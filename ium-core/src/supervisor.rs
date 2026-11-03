//! The supervisor (issue #15): spawn/pause/resume/cancel over a
//! [`JobManager`]. This is the layer the desktop shell drives — it lives in
//! `ium-core` so every semantic is testable without a webview; the Tauri
//! glue stays thin (state extraction + event emission only).
//!
//! Rules it keeps:
//! * `drive()` is idempotent per job — one driver task at a time (the
//!   manager's single-driver guard stays the backstop).
//! * The journal is the truth: a finished/failed/canceled task removes
//!   itself; UIs poll [`JobManager::snapshot`] (that IS the `job_updated`
//!   stream's content) — nothing here caches state.
//! * `cancel()` journals the Canceled transition BEFORE the drive task is
//!   aborted (ADR-002): a crash between the two still leaves an honest,
//!   terminal journal.
//! * `resume_signal()` re-spawns the drive task — the old one exited when it
//!   journaled Paused; a flag flip alone would resurrect nothing.
//! * Drive tasks spawn through a runtime [`Handle`] captured at
//!   construction (or given explicitly via [`Supervisor::with_handle`]) —
//!   NEVER through the ambient context. The shell runs `create_job` /
//!   `resume_signal` as sync tauri commands on the GTK main thread, where
//!   no tokio context is entered; a raw `tokio::spawn` there panics and,
//!   unwinding across the webview IPC FFI boundary, aborts the whole
//!   shell (issue #39, CI #65–#70: "Session terminated without a reply").

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::manager::JobManager;
use crate::{JobRecord, Result};

struct Inner {
    manager: Arc<JobManager>,
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    /// The runtime drive tasks spawn on: captured from the construction
    /// context when there is one, given explicitly by hosts that construct
    /// off-runtime (the tauri shell's setup runs on the main thread).
    runtime: Option<Handle>,
}

/// Spawn/pause/resume/cancel over one shared [`JobManager`] — the engine
/// side of the shell's runtime wiring (#15). All semantics live here so
/// they are testable without a webview; Tauri glue stays thin.
#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

impl Supervisor {
    /// Wrap a manager; drive tasks are spawned per job on demand. Captures
    /// the current runtime context when one is entered; hosts that construct
    /// off-runtime (the tauri shell) must use [`Supervisor::with_handle`].
    pub fn new(manager: Arc<JobManager>) -> Self {
        Self::with_handle_opt(manager, Handle::try_current().ok())
    }

    /// Wrap a manager, naming the runtime drive tasks spawn on. For hosts
    /// whose construction thread has no runtime context (tauri setup runs
    /// on the main thread — issue #39): pass `tauri::async_runtime`'s tokio
    /// handle and every later `drive()` works from any thread.
    pub fn with_handle(manager: Arc<JobManager>, runtime: Handle) -> Self {
        Self::with_handle_opt(manager, Some(runtime))
    }

    fn with_handle_opt(manager: Arc<JobManager>, runtime: Option<Handle>) -> Self {
        Self {
            inner: Arc::new(Inner {
                manager,
                tasks: Mutex::new(HashMap::new()),
                runtime,
            }),
        }
    }

    /// The wrapped manager (for create_job/snapshot/cancel plumbing).
    pub fn manager(&self) -> &Arc<JobManager> {
        &self.inner.manager
    }

    /// Is a drive task currently alive for this job? (Journal state remains
    /// the authority for anything user-visible; this only gates re-spawns.)
    pub fn is_driving(&self, job_id: &str) -> bool {
        self.inner
            .tasks
            .lock()
            .unwrap()
            .get(job_id)
            .is_some_and(|h| !h.is_finished())
    }

    /// Spawn the drive task for `job_id` (idempotent while one is alive).
    /// The task removes itself when `run_job` returns — Done, Failed,
    /// Paused, Canceled or error: the journal holds whatever is true.
    ///
    /// Callable from ANY thread: the spawn goes through the runtime handle
    /// captured at construction (or given to [`Supervisor::with_handle`]),
    /// falling back to the ambient context only when no handle is stored.
    /// Panics honestly only when a host constructed context-less, gave no
    /// handle, and drives from context-less code — a host bug, not an
    /// ambient accident (issue #39).
    pub fn drive(&self, job_id: &str) {
        let mut tasks = self.inner.tasks.lock().unwrap();
        if tasks.get(job_id).is_some_and(|h| !h.is_finished()) {
            return;
        }
        let runtime = self
            .inner
            .runtime
            .clone()
            .or_else(|| Handle::try_current().ok())
            .expect(
                "Supervisor::drive called with no runtime: construct with \
                 Supervisor::with_handle (runtime-less hosts) or call from \
                 a tokio runtime context",
            );
        let manager = Arc::clone(&self.inner.manager);
        let inner = Arc::clone(&self.inner);
        let id = job_id.to_string();
        let handle = runtime.spawn(async move {
            let _ = manager.run_job(&id).await;
            inner.tasks.lock().unwrap().remove(&id);
        });
        tasks.insert(job_id.to_string(), handle);
    }

    /// Ask the manager to pause (cooperative; takes effect between leases).
    /// Errors on unknown/terminal jobs.
    pub fn pause(&self, job_id: &str) -> Result<()> {
        self.inner.manager.pause(job_id)
    }

    /// Resume: clear the pause flag AND re-spawn the drive task (the previous
    /// task exited when it journaled Paused — a flag flip alone is a no-op).
    pub fn resume_signal(&self, job_id: &str) -> Result<()> {
        self.inner.manager.resume_signal(job_id)?;
        self.drive(job_id);
        Ok(())
    }

    /// Cancel: journal Canceled FIRST (the manager does), then abort the
    /// drive task. The abort is a backstop for in-flight adapter I/O — the
    /// drive loop itself exits promptly on the cancel flag without it.
    pub async fn cancel(&self, job_id: &str) -> Result<JobRecord> {
        let record = self.inner.manager.cancel(job_id).await?;
        if let Some(handle) = self.inner.tasks.lock().unwrap().remove(job_id) {
            handle.abort();
        }
        Ok(record)
    }
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor").finish_non_exhaustive()
    }
}
