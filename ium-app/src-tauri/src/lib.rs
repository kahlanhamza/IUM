//! IUM desktop shell — thin Tauri 2 layer over `ium-core`.
//!
//! Rule (spec §2.3): the UI never computes transfer truth. Commands only
//! forward to the JobManager; the SQLite journal is the single source of truth.

use std::sync::Mutex;

use ium_core::{JobManager, JobRecord, JobSnapshot, JobSpec};
use tauri::{Manager, State};

struct Engine(Mutex<JobManager>);

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

#[tauri::command]
fn engine_status() -> String {
    format!("ium-core v{} ready", ium_core::VERSION)
}

#[tauri::command]
fn create_job(engine: State<Engine>, spec: JobSpec, max_conns: u32) -> Result<JobRecord, String> {
    engine
        .0
        .lock()
        .unwrap()
        .create_job(&spec, max_conns)
        .map_err(engine_error)
}

#[tauri::command]
fn list_jobs(engine: State<Engine>) -> Result<Vec<JobRecord>, String> {
    engine.0.lock().unwrap().list_jobs().map_err(engine_error)
}

#[tauri::command]
fn snapshot(engine: State<Engine>, job_id: String) -> Result<JobSnapshot, String> {
    engine
        .0
        .lock()
        .unwrap()
        .snapshot(&job_id)
        .map_err(engine_error)
}

#[tauri::command]
fn pause_job(engine: State<Engine>, job_id: String) -> Result<(), String> {
    engine
        .0
        .lock()
        .unwrap()
        .pause(&job_id)
        .map_err(engine_error)
}

#[tauri::command]
fn resume_signal(engine: State<Engine>, job_id: String) -> Result<(), String> {
    engine
        .0
        .lock()
        .unwrap()
        .resume_signal(&job_id)
        .map_err(engine_error)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let manager = JobManager::open(db_path(app.handle())).map_err(|e| e.to_string())?;
            app.manage(Engine(Mutex::new(manager)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            create_job,
            list_jobs,
            snapshot,
            pause_job,
            resume_signal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running IUM");
}
