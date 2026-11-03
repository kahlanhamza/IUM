//! IUM desktop shell — thin Tauri 2 layer over `ium-core`.
//!
//! Rule (spec §2.3): the UI never computes transfer truth. Commands only
//! forward to the Supervisor/JobManager; the SQLite journal is the single
//! source of truth. `job_updated` events carry the same journal snapshots a
//! poll would return — the emitter READS the journal, it never derives.
//!
//! Issue #15: the shell actually uploads. `create_job` drives the job
//! through the engine's `Supervisor` (spawn/cancel/pause/resume semantics
//! live in `ium-core::supervisor`, testable without a webview); `cancel`
//! journals the honest `Canceled` terminal state BEFORE the drive task is
//! aborted.

use std::sync::Arc;
use std::time::Duration;

use ium_core::{
    HttpOneShotAdapter, JobManager, JobRecord, JobSnapshot, JobSpec, S3MultipartAdapter,
    S3MultipartConfig, Supervisor, TusAdapter,
};
use tauri::{Emitter, Manager, State};

struct Engine(Arc<Supervisor>);

/// Registry names for the create-job form's adapter select (#16): the UI
/// reflects the engine's registry, it never hard-codes protocol lists.
#[tauri::command]
fn list_adapters(engine: State<Engine>) -> Vec<String> {
    engine
        .0
        .manager()
        .adapter_names()
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ium-session.db")
}

fn engine_error(e: ium_core::EngineError) -> String {
    e.to_string()
}

/// Register the protocol adapters this build ships with. S3 credentials
/// come from the environment (`S3MultipartConfig::from_env`); a machine
/// without them simply has no `s3_multipart` in the registry — the UI
/// reflects the registry, it never assumes.
fn register_adapters(manager: &mut JobManager) {
    manager.register_adapter(Arc::new(HttpOneShotAdapter::new().expect("http client")));
    manager.register_adapter(Arc::new(TusAdapter::new()));
    match S3MultipartConfig::from_env() {
        Ok(config) => match S3MultipartAdapter::new(config) {
            Ok(adapter) => manager.register_adapter(Arc::new(adapter)),
            Err(e) => tracing::warn!("s3_multipart unavailable: {e}"),
        },
        Err(e) => tracing::warn!("s3_multipart not configured (no env credentials): {e}"),
    }
}

#[tauri::command]
fn engine_status(engine: State<Engine>) -> String {
    format!(
        "ium-core v{} ready — adapters: {}",
        ium_core::VERSION,
        engine.0.manager().adapter_names().join(", ")
    )
}

#[tauri::command]
fn create_job(
    engine: State<Engine>,
    app: tauri::AppHandle,
    spec: JobSpec,
    max_conns: u32,
) -> Result<JobRecord, String> {
    let record = engine
        .0
        .manager()
        .create_job(&spec, max_conns)
        .map_err(engine_error)?;
    engine.0.drive(&record.id);
    spawn_watcher(app, engine.0.clone(), record.id.clone());
    Ok(record)
}

#[tauri::command]
fn list_jobs(engine: State<Engine>) -> Result<Vec<JobRecord>, String> {
    engine.0.manager().list_jobs().map_err(engine_error)
}

#[tauri::command]
fn snapshot(engine: State<Engine>, job_id: String) -> Result<JobSnapshot, String> {
    engine.0.manager().snapshot(&job_id).map_err(engine_error)
}

/// Spec Sprint-1 item 9 name for the per-chunk inspector drawer (#16).
#[tauri::command]
fn inspect_chunks(engine: State<Engine>, job_id: String) -> Result<JobSnapshot, String> {
    engine.0.manager().snapshot(&job_id).map_err(engine_error)
}

#[tauri::command]
fn pause_job(engine: State<Engine>, job_id: String) -> Result<(), String> {
    engine.0.pause(&job_id).map_err(engine_error)
}

#[tauri::command]
fn resume_signal(
    engine: State<Engine>,
    app: tauri::AppHandle,
    job_id: String,
) -> Result<(), String> {
    // Re-drives the job (the old task exited when it journaled Paused) and
    // restarts the event watcher for it.
    engine.0.resume_signal(&job_id).map_err(engine_error)?;
    spawn_watcher(app, engine.0.clone(), job_id);
    Ok(())
}

/// Cancel: journaled `Canceled` FIRST, then the drive task is aborted —
/// a user cancel is never recorded as `Failed` (honest endings, issue #15).
#[tauri::command]
async fn cancel_job(engine: State<'_, Engine>, job_id: String) -> Result<JobRecord, String> {
    engine.0.cancel(&job_id).await.map_err(engine_error)
}

/// The create-job dialog needs the size the engine's JobSpec requires; the
/// shell stats, the engine plans (the UI never computes transfer truth).
#[tauri::command]
fn stat_file(path: String) -> Result<u64, String> {
    std::fs::metadata(&path)
        .map(|m| m.len())
        .map_err(|e| format!("{}: {e}", path))
}

/// `job_updated` stream: journal snapshots on a fixed interval until the
/// job is terminal (the terminal snapshot is emitted, then the watcher
/// stops — a finished job generates no events forever).
fn spawn_watcher(app: tauri::AppHandle, sup: Arc<Supervisor>, job_id: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let Ok(snap) = sup.manager().snapshot(&job_id) else {
                return;
            };
            let terminal = snap.job.state.is_terminal();
            let _ = app.emit("job_updated", &snap);
            if terminal {
                return;
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let manager = JobManager::open(db_path(app.handle())).map_err(|e| e.to_string())?;
            let mut manager = manager;
            register_adapters(&mut manager);
            // Issue #39: setup runs on the main thread — the same thread
            // tauri runs SYNC commands on — and no tokio context is entered
            // there. The supervisor must spawn drive tasks through an
            // explicit handle (tauri's async runtime, whose tokio handle we
            // read from inside a block_on-entered context); a raw
            // tokio::spawn in create_job/resume_signal panicked here and
            // aborted the whole shell on the first Upload click
            // ("Session terminated without a reply", CI #65–#70).
            let runtime_handle =
                tauri::async_runtime::block_on(async { tokio::runtime::Handle::current() });
            app.manage(Engine(Arc::new(Supervisor::with_handle(
                Arc::new(manager),
                runtime_handle,
            ))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            list_adapters,
            create_job,
            list_jobs,
            snapshot,
            inspect_chunks,
            pause_job,
            resume_signal,
            cancel_job,
            stat_file,
        ])
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running IUM");
}
