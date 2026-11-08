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
//! * A bounded supervisor (lane 5, product bar #66) admits at most `max`
//!   simultaneous drive tasks; the rest FIFO-queue and are driven when a
//!   slot frees. Queued jobs hold no resources beyond a queue entry; the
//!   journal stays the only user-visible truth (a queued job reads
//!   Created/whatever the journal says — nothing new to poll).
//! * Drive tasks spawn through a runtime [`Handle`] captured at
//!   construction (or given explicitly via [`Supervisor::with_handle`]) —
//!   NEVER through the ambient context. The shell runs `create_job` /
//!   `resume_signal` as sync tauri commands on the GTK main thread, where
//!   no tokio context is entered; a raw `tokio::spawn` there panics and,
//!   unwinding across the webview IPC FFI boundary, aborts the whole
//!   shell (issue #39, CI #65–#70: "Session terminated without a reply").

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::manager::JobManager;
use crate::state::JobState;
use crate::{JobRecord, Result};

/// All drive-slot state under ONE lock: the live task map plus, when the
/// supervisor is bounded, the FIFO queue of jobs waiting for a slot. The
/// capacity check and the pop must be atomic with the push (`drive()`
/// racing a completing task must never lose a wakeup), so they share one
/// mutex. Lock ordering: `drives` is a leaf — no manager/journal lock is
/// ever taken before it and released while held.
struct Drives {
    tasks: HashMap<String, JoinHandle<()>>,
    queue: VecDeque<String>,
}

struct Inner {
    manager: Arc<JobManager>,
    drives: Mutex<Drives>,
    /// `None` = unbounded (the legacy semantics, byte-for-byte); `Some(n)`
    /// admits at most `n` simultaneous drive tasks, FIFO-queueing the rest.
    max_concurrent: Option<usize>,
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
        Self::with_handle_opt(manager, Handle::try_current().ok(), None)
    }

    /// Wrap a manager, naming the runtime drive tasks spawn on. For hosts
    /// whose construction thread has no runtime context (tauri setup runs
    /// on the main thread — issue #39): pass `tauri::async_runtime`'s tokio
    /// handle and every later `drive()` works from any thread.
    pub fn with_handle(manager: Arc<JobManager>, runtime: Handle) -> Self {
        Self::with_handle_opt(manager, Some(runtime), None)
    }

    /// Wrap a manager with a bounded admission: at most `max` drive tasks
    /// run simultaneously; further `drive()` calls FIFO-queue their job and
    /// it is driven when a slot frees (lane 5 of product bar #66 — a
    /// 100-file intake must not be 100 simultaneous uploads). `max = 0` is
    /// an honest panic: "bounded" that bounds nothing is a lie.
    pub fn with_concurrency_limit(manager: Arc<JobManager>, max: usize) -> Self {
        Self::checked_limit(max);
        Self::with_handle_opt(manager, Handle::try_current().ok(), Some(max))
    }

    /// [`Self::with_concurrency_limit`] for hosts that construct
    /// off-runtime (the tauri shell): pass the runtime handle explicitly
    /// (issue #39) and the limit together.
    pub fn with_handle_and_limit(manager: Arc<JobManager>, runtime: Handle, max: usize) -> Self {
        Self::checked_limit(max);
        Self::with_handle_opt(manager, Some(runtime), Some(max))
    }

    fn checked_limit(max: usize) {
        assert!(
            max >= 1,
            "Supervisor concurrency limit must be >= 1, got {max}"
        );
    }

    fn with_handle_opt(
        manager: Arc<JobManager>,
        runtime: Option<Handle>,
        max_concurrent: Option<usize>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                manager,
                drives: Mutex::new(Drives {
                    tasks: HashMap::new(),
                    queue: VecDeque::new(),
                }),
                max_concurrent,
                runtime,
            }),
        }
    }

    /// The wrapped manager (for create_job/snapshot/cancel plumbing).
    pub fn manager(&self) -> &Arc<JobManager> {
        &self.inner.manager
    }

    /// Is a drive task currently alive for this job? (Journal state remains
    /// the authority for anything user-visible; this only gates re-spawns.
    /// A job waiting in the limiter queue is NOT driving.)
    pub fn is_driving(&self, job_id: &str) -> bool {
        self.inner
            .drives
            .lock()
            .unwrap()
            .tasks
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
        let mut drives = self.inner.drives.lock().unwrap();
        if drives.tasks.get(job_id).is_some_and(|h| !h.is_finished()) {
            return;
        }
        // Bounded and full → FIFO-queue and walk away (the shell contract:
        // one call, no waiting). A slot release pops this job later. The
        // dedup keeps `resume_signal`/double-`drive` from double-queuing.
        if let Some(max) = self.inner.max_concurrent {
            if drives.tasks.len() >= max {
                if !drives.queue.contains(&job_id.to_string()) {
                    drives.queue.push_back(job_id.to_string());
                }
                return;
            }
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
        self.spawn_drive(&mut drives, &runtime, job_id);
    }

    /// Insert the drive task for `job_id` (caller holds the drives lock and
    /// has already decided a slot is available).
    fn spawn_drive(&self, drives: &mut Drives, runtime: &Handle, job_id: &str) {
        let manager = Arc::clone(&self.inner.manager);
        let inner = Arc::clone(&self.inner);
        let id = job_id.to_string();
        let rt = runtime.clone();
        let handle = runtime.spawn(async move {
            let _ = manager.run_job(&id).await;
            release_and_pop(&inner, &rt, &id);
        });
        drives.tasks.insert(job_id.to_string(), handle);
    }

    /// Ask the manager to pause (cooperative; takes effect between leases).
    /// Errors on unknown/terminal jobs.
    pub fn pause(&self, job_id: &str) -> Result<()> {
        self.inner.manager.pause(job_id)
    }

    /// The global upload cap (#94): bytes/sec across ALL jobs and lanes, or
    /// `None` for unlimited. Hot — running workers obey on their next
    /// between-lease acquire. The shell persists the user's choice and
    /// re-asserts it at boot (a policy knob, not journal truth).
    pub fn set_rate_cap(&self, cap: Option<u64>) {
        self.inner.manager.set_rate_cap(cap);
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
    /// A cancel-while-queued job leaves the queue at once (and would be
    /// dropped at pop time anyway — Canceled is terminal).
    pub async fn cancel(&self, job_id: &str) -> Result<JobRecord> {
        let record = self.inner.manager.cancel(job_id).await?;
        let mut drives = self.inner.drives.lock().unwrap();
        if let Some(handle) = drives.tasks.remove(job_id) {
            handle.abort();
        }
        drives.queue.retain(|id| id != job_id);
        Ok(record)
    }
}

/// A drive task finished: free its slot and, when bounded, pop the queue —
/// skipping entries that turned terminal while queued (canceled/failed —
/// the journal is the truth) — until one live job fills the slot. Runs on
/// the runtime (inside the completing task), so the spawn context exists.
fn release_and_pop(inner: &Arc<Inner>, runtime: &Handle, finished_id: &str) {
    let mut drives = inner.drives.lock().unwrap();
    drives.tasks.remove(finished_id);
    let Some(max) = inner.max_concurrent else {
        return;
    };
    while drives.tasks.len() < max {
        let Some(next) = drives.queue.pop_front() else {
            break;
        };
        // Terminal-while-queued → drop without driving. An unknown id is
        // treated the same (the journal never knew it — nothing to drive).
        let state = inner
            .manager
            .snapshot(&next)
            .map(|s| s.job.state)
            .unwrap_or(JobState::Failed);
        if matches!(
            state,
            JobState::Done | JobState::Failed | JobState::Unresumable | JobState::Canceled
        ) {
            continue;
        }
        let manager = Arc::clone(&inner.manager);
        let inner2 = Arc::clone(inner);
        let rt = runtime.clone();
        let rt2 = rt.clone();
        let next_for_task = next.clone();
        let handle = rt.spawn(async move {
            let _ = manager.run_job(&next_for_task).await;
            release_and_pop(&inner2, &rt2, &next_for_task);
        });
        drives.tasks.insert(next, handle);
        break;
    }
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor").finish_non_exhaustive()
    }
}
