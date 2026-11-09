//! JobManager — the orchestrator facade (pattern #5: supervised engine,
//! in-process). Every state transition is journaled before it executes;
//! `bytes_confirmed` only ever carries server acknowledgments.
//!
//! Guarantees enforced here:
//! * the state machine is validated against the state **in the journal**, not
//!   against caller-supplied arguments — the journal is the truth;
//! * a partially-confirmed chunk goes back to `Pending` keeping its confirmed
//!   bytes, so it is re-leased with `resume_from` — never re-uploaded from
//!   zero, never skipped;
//! * retry attempts are counted per chunk — a hostile endpoint cannot wedge
//!   the supervisor in an infinite retry loop;
//! * a job has at most one active driver — double `run_job` is an error, not
//!   a race;
//! * one-shot (non-resumable) endpoints are planned as a single chunk — there
//!   is nothing to resume, so chunking would be a lie.
//!
//! Concurrency model (Sprint 1): a drive round spawns up to
//! up to `MAX_WORKERS` cooperative workers over ONE shared [`ChunkPool`]. Each
//! worker leases, uploads (streaming progress reported per window), and
//! journals its own chunks; a fatal error aborts the fleet via a watch
//! channel and the driver journals `Failed` exactly once. Pausing is
//! cooperative: it takes effect after the in-flight chunk completes, because
//! for a one-shot endpoint the in-flight bytes cannot be re-sent.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use tokio::sync::watch;

use crate::adapter::{
    AdapterError, AdapterRegistry, ChunkConfirm, ProgressReporter, TransferAdapter,
};
use crate::chunk::{ChunkPool, ChunkState};
use crate::eviction::{below_floor, median, EvictionPolicy, ResplitPolicy, ThroughputWatch};
use crate::journal::{JobRecord, SessionJournal};
use crate::retry::{Action, RetryPolicy};
use crate::state::JobState;
use crate::{EngineError, JobSpec, Result};

/// Sanity ceiling on workers per drive round; the chunk plan (create_job
/// plans one chunk per configured connection, spec §4.1) normally binds first.
const MAX_WORKERS: usize = 32;

/// Issue #47: bound on consecutive round-end strand reclaims that make ZERO
/// unconfirmed-byte progress. One reclaim round is the honest self-heal (the
/// drill's shape: a stranded lease returns to the pool and completes); three
/// in a row means a driver bug is stranding leases deterministically —
/// snapshot the pool and break to the loud verify gate instead of spinning.
const MAX_IDLE_RECLAIM_ROUNDS: u32 = 3;

/// Issue #49: the evidence window — how often the driver's ticker folds
/// completed-call evidence into a fleet decision. 1 s: long enough to
/// collect several calls on any real link, short enough to follow the
/// owner's 54x swing within seconds.
const EVIDENCE_WINDOW: Duration = Duration::from_secs(1);

/// Issue #49: how often a PARKED worker re-checks the pool on its own (the
/// growth watch is push-driven; the poll only guarantees that a parked
/// worker notices pool exhaustion and exits through the normal path).
const PARK_POLL: Duration = Duration::from_millis(250);

/// UI-facing snapshot of a job (spec §8 transparency): the journal's job
/// row, its chunk rows and its event trail, read fresh — the UI NEVER
/// computes transfer state itself (spec §2.3).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobSnapshot {
    /// The job's journal record (state included — journal truth).
    pub job: JobRecord,
    /// Per-chunk rows in idx order (the inspector drawer's table).
    pub chunks: Vec<crate::journal::ChunkRow>,
    /// The append-only event trail `(seq, kind, payload_json)`.
    pub events: Vec<(i64, String, String)>,
}

/// The orchestrator facade: drives jobs over the registry's adapters with
/// the journal as the only source of truth (ADR-002). Cooperative workers
/// lease from one shared [`crate::chunk::ChunkPool`]; every transition is
/// journaled before it executes.
pub struct JobManager {
    journal: Mutex<SessionJournal>,
    registry: AdapterRegistry,
    policy: RetryPolicy,
    eviction: EvictionPolicy,
    /// Pattern #9 tuning (issue #27): when to reshape the pending plan from
    /// measured throughput. See [`ResplitPolicy`] — the honest goal is
    /// feedback latency, not bandwidth.
    resplit: ResplitPolicy,
    /// job_id -> pause flag (watch keeps the last value; workers observe it).
    pause_flags: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// job_id -> cancel flag (issue #15). The flag is a request signal; the
    /// JOURNAL transition (made by `cancel()` BEFORE the flag flips) is the
    /// truth the drive loop obeys.
    cancel_flags: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// job ids with an active driver task — enforces single-driver.
    active: Mutex<HashSet<String>>,
    /// The global rate bucket (#94): every worker of every job acquires its
    /// lease's bytes here BEFORE the upload — the cap is the SUM across
    /// parallel flows. Unlimited (`None`) = the acquire is a no-op.
    rate: Arc<crate::rate::Bucket>,
}

/// RAII guard so a panicking/early-returning driver always releases its slot.
struct ActiveGuard<'a> {
    mgr: &'a JobManager,
    id: String,
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.mgr.active.lock().unwrap().remove(&self.id);
    }
}

/// State shared by the worker fleet of one drive round. Every field is either
/// immutable for the round or mutex/atomic-guarded; no lock is ever held
/// across an await (SQLite ops are sub-millisecond, pool ops are arithmetic).
struct DriveShared {
    pool: Arc<Mutex<ChunkPool>>,
    /// Per-chunk retry budget (errors + evictions), shared across workers.
    attempts: Arc<Mutex<HashMap<u32, u32>>>,
    /// idx → last sampled wire throughput (bytes/sec) per leased chunk; the
    /// eviction floor is the median of these live samples.
    live_bps: Arc<Mutex<HashMap<u32, f64>>>,
    /// idx → one vote per COMPLETED lease (issue #25): the lease's freshest
    /// measured rate — the last sampled window bps when one elapsed, else the
    /// lease mean. Finished chunks keep voting so the floor stays honest when
    /// the fleet drains (the lone-impaired-worker case the #22 S3 drill
    /// found: every peer confirmed sub-window, the laggard became the only
    /// sample, `below_floor` refused to judge, and the wait-out ran ~472 s).
    done_bps: Arc<Mutex<HashMap<u32, f64>>>,
    /// The fleet's best LIVE window rate this drive round (f64 bit pattern; 0
    /// = no window has closed yet). Completed-lease votes are capped at it:
    /// a sub-window mean may not outrun any rate the round actually sampled,
    /// or the inflated median would evict HEALTHY streamers into budget
    /// exhaustion — caught by the integration eviction drill on the first
    /// draft of #25. Non-negative f64 bit patterns order like the values, so
    /// a plain integer fetch_max is a float max.
    best_live_bps: Arc<AtomicU64>,
    abort_tx: watch::Sender<bool>,
    abort_rx: watch::Receiver<bool>,
    /// Set by the worker that hit a fatal error; the driver journals Failed.
    failed: Arc<AtomicBool>,
    /// Set by workers that observed the pause flag; the driver journals Paused.
    paused_seen: Arc<AtomicBool>,
    paused_rx: watch::Receiver<bool>,
    /// Issue #15: the journal already holds Canceled before this flips; the
    /// nap and the loop top just obey it promptly.
    cancel_rx: watch::Receiver<bool>,
    /// Issue #27: bumped (epoch+1) by the single resplit winner BEFORE the
    /// pool is reshaped; every peer's `select!` watches it and drops its
    /// in-flight call (its lease was just retired) instead of letting a
    /// stale confirm/error land on the renumbered plan.
    resplit_rx: watch::Receiver<u64>,
    /// Issue #27: the epoch sender (cloned into DriveShared so the winning
    /// worker can bump it; peers hold only receivers).
    resplit_tx: watch::Sender<u64>,
    /// Issue #27 single-flight gate: only one worker per round may execute
    /// a resplit; every other proposer re-checks on its next window.
    resplit_in_flight: Arc<AtomicBool>,
    /// Issue #28 frozen-link rule: when the job's FIRST lease began — the
    /// starvation clock. A fleet median of 0 (every sample zero: the
    /// adapter's writes never complete into buffers this small) is
    /// unmeasurable for `plan()`, but a job starving past `slow_confirm`
    /// with an oversized part still outstanding is data: the part is too
    /// coarse. Per-lease elapsed cannot serve here (a hostile server kills
    /// every attempt at its own 30 s read deadline — cloud RED
    /// 41571828134 — resetting the clock each retry); the JOB is the unit
    /// that starves.
    first_lease_started: Arc<Mutex<Option<Instant>>>,
    /// Issue #49 (M8 rung 2): the LIVE fleet waterline. Workers at or above
    /// it park at their lease boundary and wake when it rises — shrink and
    /// growth both take effect MID-round. Seeded journal-first (#48), moved
    /// by the driver's evidence ticker.
    fleet_rx: watch::Receiver<u32>,
    /// Issue #49: completed-call evidence — (Instant, server-confirmed
    /// bytes, counted error) per finished upload call. The ticker drains it
    /// once per window; Busy waits and stale drops record nothing (a wait
    /// is not a completed call, a stranger's error is not ours).
    evidence: Arc<Mutex<Vec<(Instant, u64, bool)>>>,
}

impl JobManager {
    /// Open (creating or migrating) the journal at `path` and build a
    /// manager over it. No adapters registered yet — see
    /// [`JobManager::register_adapter`].
    pub fn open(journal_path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            journal: Mutex::new(SessionJournal::open(journal_path).map_err(EngineError::from)?),
            registry: AdapterRegistry::new(),
            policy: RetryPolicy::default(),
            eviction: EvictionPolicy::default(),
            resplit: ResplitPolicy::default(),
            pause_flags: Mutex::new(HashMap::new()),
            cancel_flags: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
            rate: Arc::new(crate::rate::Bucket::new(None)),
        })
    }

    /// Build a manager over an already-open journal (tests, drills, and
    /// any host that owns the journal's lifecycle).
    pub fn with_journal(journal: SessionJournal) -> Result<Self> {
        Ok(Self {
            journal: Mutex::new(journal),
            registry: AdapterRegistry::new(),
            policy: RetryPolicy::default(),
            eviction: EvictionPolicy::default(),
            resplit: ResplitPolicy::default(),
            pause_flags: Mutex::new(HashMap::new()),
            cancel_flags: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
            rate: Arc::new(crate::rate::Bucket::new(None)),
        })
    }

    /// Register an adapter under its [`TransferAdapter::name`]; `JobSpec`
    /// adapter keys resolve here. Register every adapter BEFORE `create_job`.
    pub fn register_adapter(&mut self, adapter: Arc<dyn TransferAdapter>) {
        self.registry.register(adapter);
    }

    /// Hot lane registration (#81): upsert by [`TransferAdapter::name`] —
    /// a KNOWN name swaps the lane's config in place (returns `true`), an
    /// UNKNOWN name appends the lane (returns `false`; the first credential
    /// entry for a lane boot never registered). In-flight jobs keep the Arc
    /// they resolved at `create_job` — the swap is never retroactive; jobs
    /// created after it resolve the fresh config. Works on `&self` because
    /// the supervisor hands the manager out as an `Arc<JobManager>` and the
    /// shell's credential commands have no `&mut` path back.
    pub fn replace_adapter(&self, adapter: Arc<dyn TransferAdapter>) -> bool {
        self.registry.replace(adapter)
    }

    /// Remove a lane by registry key (#81: removing its credentials removes
    /// the lane — "a machine without credentials for a lane simply has no
    /// such entry in the registry"). Returns whether a lane was removed.
    /// Same non-retroactive contract as [`JobManager::replace_adapter`].
    pub fn remove_adapter(&self, name: &str) -> bool {
        self.registry.remove(name)
    }

    /// Replace the retry policy (spec §5's per-error-class decisions).
    pub fn set_policy(&mut self, policy: RetryPolicy) {
        self.policy = policy;
    }

    /// The global upload cap (#94): bytes/sec across ALL jobs and lanes —
    /// the sum the fleet may take — or `None` for unlimited. A policy knob
    /// (the set_policy family's shape): runtime state, not journal truth;
    /// the shell persists the user's choice and re-asserts it at boot. HOT:
    /// running workers obey on their next between-lease acquire.
    pub fn set_rate_cap(&self, cap: Option<u64>) {
        self.rate.set_rate(cap);
    }

    /// The bucket reference for the worker loop's pre-upload acquire.
    fn rate(&self) -> &Arc<crate::rate::Bucket> {
        &self.rate
    }

    /// Slow-chunk eviction tuning (spec §4.2). Shrink the window in tests.
    pub fn set_eviction_policy(&mut self, eviction: EvictionPolicy) {
        self.eviction = eviction;
    }

    /// Adaptive-resplit tuning (issue #27). Shrink the thresholds in tests.
    pub fn set_resplit_policy(&mut self, resplit: ResplitPolicy) {
        self.resplit = resplit;
    }

    /// Registry keys, for diagnostics and the shell's status line.
    pub fn adapter_names(&self) -> Vec<&'static str> {
        self.registry.names()
    }

    /// Journal access (diagnostics, tests, UI replay). Guard derefs to `SessionJournal`.
    pub fn journal(&self) -> std::sync::MutexGuard<'_, SessionJournal> {
        self.journal.lock().expect("journal mutex poisoned")
    }

    fn j(&self) -> std::sync::MutexGuard<'_, SessionJournal> {
        self.journal.lock().expect("journal mutex poisoned")
    }

    fn adapter(&self, spec: &JobSpec) -> Result<Arc<dyn TransferAdapter>> {
        self.registry
            .get(&spec.adapter)
            .ok_or_else(|| EngineError::UnknownAdapter(spec.adapter.clone()))
    }

    /// Create + journal a job (state `Created`, chunk plan persisted).
    /// Non-resumable endpoints get exactly one chunk: chunking a stream that
    /// cannot resume would imply a capability the protocol does not have.
    pub fn create_job(&self, spec: &JobSpec, max_conns: u32) -> Result<JobRecord> {
        let adapter = self.adapter(spec)?;
        let cap = adapter.capabilities();
        let id = crate::ids::new_id();
        let workers = if cap.has(crate::adapter::Capability::RESUMABLE) {
            max_conns
        } else {
            1
        };
        let pool = match adapter.uniform_block() {
            // Issue #56: a prescribed-geometry lane is BORN uniform-tail —
            // the wire rejects every other shape (Feishu 1062009), so the
            // create-time plan must already be the prescribed grid.
            Some(block) => ChunkPool::new_uniform(spec.source_size, block),
            None => ChunkPool::new(spec.source_size, workers),
        };
        let record = self
            .j()
            .insert_job(crate::journal::NewJob {
                id: &id,
                source_path: &spec.source_path,
                source_size: spec.source_size,
                adapter: &spec.adapter,
                endpoint: &spec.endpoint,
                capability_badge: cap.badge(),
                chunks: &pool.rows(),
            })
            .map_err(EngineError::from)?;
        self.j()
            .append_event(
                &id,
                "job_created",
                &serde_json::json!({
                    "adapter": spec.adapter,
                    "endpoint": spec.endpoint,
                    "chunks": pool.chunks().len(),
                    "badge": cap.badge(),
                    // Issue #50: the REQUESTED concurrency, journaled — the
                    // create-time plan is CHUNK_MIN-floored (a 768 KiB job
                    // plans one 5 MiB-shaped chunk), so the probe's fleet
                    // seed cannot read the intent off the pool. Pre-#50
                    // journals lack the key and fall back to the pool.
                    "max_conns": workers,
                }),
            )
            .map_err(EngineError::from)?;
        Ok(record)
    }

    /// Full snapshot for the UI.
    pub fn snapshot(&self, job_id: &str) -> Result<JobSnapshot> {
        let job = self
            .j()
            .load_job(job_id)
            .map_err(EngineError::from)?
            .ok_or_else(|| EngineError::JobNotFound(job_id.to_string()))?;
        let chunks = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        let events = self.j().events_for(job_id).map_err(EngineError::from)?;
        Ok(JobSnapshot {
            job,
            chunks,
            events,
        })
    }

    /// All job rows, newest first (the UI's job list).
    pub fn list_jobs(&self) -> Result<Vec<JobRecord>> {
        self.j().list_jobs().map_err(EngineError::from)
    }

    /// Ask the supervisor to pause between chunks (cooperative).
    pub fn pause(&self, job_id: &str) -> Result<()> {
        let job = self.require_job(job_id)?;
        if job.state.is_terminal() {
            return Err(EngineError::WrongState(
                job_id.to_string(),
                job.state.as_str().to_string(),
                "non-terminal",
            ));
        }
        self.ensure_pause_flag(job_id).send_replace(true);
        self.j()
            .append_event(job_id, "pause_requested", &serde_json::json!({}))
            .map_err(EngineError::from)?;
        Ok(())
    }

    /// Clear the pause flag (cooperative resume): the next drive loop —
    /// whether still parked mid-round or re-driven by the host — continues
    /// leasing. Does not start a drive task by itself (see `Supervisor`).
    pub fn resume_signal(&self, job_id: &str) -> Result<()> {
        self.require_job(job_id)?;
        if let Some(tx) = self.pause_flags.lock().unwrap().get(job_id) {
            tx.send_replace(false);
        }
        Ok(())
    }

    /// User cancel (issue #15): a first-class terminal state, never recorded
    /// as `Failed`. The journal transition happens FIRST (ADR-002: states
    /// are journaled before they execute — here "execute" is the flag flip
    /// and the supervisor's task abort), then the cancel flag wakes every
    /// worker and drive-round loop point. Legal from Created/Probing/Running/
    /// Paused and — since issue #26 — from Finishing: the verify gate is real
    /// network I/O on a KB/s uplink, and the gate honors the cancel at its
    /// entry (before any verify/Complete I/O), so a canceled upload never
    /// completes the object. NOT from any terminal state.
    pub async fn cancel(&self, job_id: &str) -> Result<JobRecord> {
        let job = self.require_job(job_id)?;
        if job.state.is_terminal() {
            return Err(EngineError::WrongState(
                job_id.to_string(),
                job.state.as_str().to_string(),
                "non-terminal",
            ));
        }
        self.transition(job_id, JobState::Canceled).await?;
        self.j()
            .append_event(job_id, "cancel_requested", &serde_json::json!({}))
            .map_err(EngineError::from)?;
        self.ensure_cancel_flag(job_id).send_replace(true);
        self.require_job(job_id)
    }

    fn ensure_cancel_flag(&self, job_id: &str) -> watch::Sender<bool> {
        self.cancel_flags
            .lock()
            .unwrap()
            .entry(job_id.to_string())
            .or_insert_with(|| watch::channel(false).0)
            .clone()
    }

    /// `transition` that tolerates the journal having moved to Canceled
    /// underneath a racing drive loop (cancel between the loop-top check
    /// and a transition): the Canceled state is the truth, return it.
    async fn transition_or_canceled(&self, job_id: &str, to: JobState) -> Result<JobState> {
        match self.transition(job_id, to).await {
            Ok(s) => Ok(s),
            Err(EngineError::IllegalTransition(e)) if e.from == JobState::Canceled => {
                Ok(JobState::Canceled)
            }
            Err(e) => Err(e),
        }
    }

    fn ensure_pause_flag(&self, job_id: &str) -> watch::Sender<bool> {
        self.pause_flags
            .lock()
            .unwrap()
            .entry(job_id.to_string())
            .or_insert_with(|| watch::channel(false).0)
            .clone()
    }

    fn require_job(&self, job_id: &str) -> Result<JobRecord> {
        self.j()
            .load_job(job_id)
            .map_err(EngineError::from)?
            .ok_or_else(|| EngineError::JobNotFound(job_id.to_string()))
    }

    /// Validate a transition against the state the JOURNAL currently holds,
    /// then journal it (design rule #2: journal-first). Idempotent no-op when
    /// the journal already sits at `to` (crash between write and action).
    /// Issue #34: the state row and its trail event commit in ONE
    /// transaction (`set_job_state_event`) — a disk-full can no longer walk
    /// between them and leave a state with no event.
    async fn transition(&self, job_id: &str, to: JobState) -> Result<JobState> {
        let from = {
            let j = self.j();
            let current = j
                .load_job(job_id)
                .map_err(EngineError::from)?
                .ok_or_else(|| EngineError::JobNotFound(job_id.to_string()))?
                .state;
            if current == to {
                return Ok(to);
            }
            current.transition(to).map_err(EngineError::from)?;
            current
        };
        self.j()
            .set_job_state_event(
                job_id,
                to,
                "state",
                &serde_json::json!({ "from": from.as_str(), "to": to.as_str() }),
            )
            .map_err(EngineError::from)?;
        tracing::info!(
            job = job_id,
            from = from.as_str(),
            to = to.as_str(),
            "transition"
        );
        Ok(to)
    }

    /// Issue #34: the honest park. When a journal WRITE fails mid-drive
    /// (ENOSPC is the canonical case — the disk that holds the truth is
    /// full), the fleet error must still leave the journal in a state a user
    /// can act on. Best-effort: atomically journal `Paused` +
    /// `journal_write_failed` (legal only from `Running`; from `Finishing`
    /// the gate is re-entered by resume, from probe states a fresh probe is
    /// harmless). If even the park cannot land (space still full — the
    /// rlimit drill's shape), the failure is logged loudly and the ORIGINAL
    /// error returned: the row keeps its last truthful state, loudness moves
    /// to the error channel. The original error is ALWAYS returned — the
    /// park is bookkeeping, never a substitute for the caller's knowledge.
    fn park_on_journal_failure(&self, job_id: &str, err: EngineError) -> EngineError {
        if !matches!(err, EngineError::Journal(_)) {
            return err;
        }
        let current = match self.require_job(job_id) {
            Ok(r) => r.state,
            Err(_) => return err, // even the read is failing; error channel carries it
        };
        if current != JobState::Running {
            // Finishing → resume re-enters the verify gate (Complete
            // idempotency, #24); Created/Probing → resume re-probes. Both
            // truthful already; no park arrow is legal or needed.
            return err;
        }
        let detail = serde_json::json!({
            "error": err.to_string(),
            "detail": "journal write failed mid-transfer; parked Paused honestly (resume after freeing space)",
        });
        match self.j().set_job_state_event(
            job_id,
            JobState::Paused,
            "journal_write_failed",
            &detail,
        ) {
            Ok(()) => {
                tracing::error!(
                    job = job_id,
                    "journal write failed mid-transfer; job parked Paused \
                     (resume after freeing space): {err}"
                );
            }
            Err(park_err) => {
                tracing::error!(
                    job = job_id,
                    "journal write failed mid-transfer AND the honest park \
                     could not be journaled ({park_err}); state stays \
                     truthful-Running, recovery via resume after freeing \
                     space: {err}"
                );
            }
        }
        err
    }

    /// Drive a job to completion. Resume-safe: rebuilds the chunk pool from
    /// the journal, so only unconfirmed work is attempted. callable from any
    /// journaled state:
    ///
    /// * `Created` / `Probing` — run the probe, then transfer;
    /// * `Paused` — resume;
    /// * `Running` / `Finishing` — crash recovery: continue where the journal
    ///   says the job was;
    /// * `Done` / `Unresumable` — terminal, returns the record unchanged;
    /// * `Failed` — error (retrying a failed job is an explicit user action:
    ///   create a new job; automatic failure-retry loops are how one bad
    ///   endpoint eats the whole session).
    pub async fn run_job(&self, job_id: &str) -> Result<JobRecord> {
        // -- single-driver guard -------------------------------------------
        if !self.active.lock().unwrap().insert(job_id.to_string()) {
            return Err(EngineError::WrongState(
                job_id.to_string(),
                "driving".to_string(),
                "a job that is not already being driven",
            ));
        }
        let _guard = ActiveGuard {
            mgr: self,
            id: job_id.to_string(),
        };

        let mut record = self.require_job(job_id)?;
        if matches!(
            record.state,
            JobState::Done | JobState::Unresumable | JobState::Canceled
        ) {
            return Ok(record);
        }
        if record.state == JobState::Failed {
            return Err(EngineError::WrongState(
                job_id.to_string(),
                record.state.as_str().to_string(),
                "a non-failed job (create a fresh job to retry)",
            ));
        }

        let spec = JobSpec {
            source_path: record.source_path.clone(),
            source_size: record.source_size,
            adapter: record.adapter.clone(),
            endpoint: record.endpoint.clone(),
        };
        let adapter = self.adapter(&spec)?;

        // -- Probing -------------------------------------------------------
        if matches!(record.state, JobState::Created | JobState::Probing) {
            if record.state == JobState::Created {
                record.state = self.transition(job_id, JobState::Probing).await?;
            }
            let probe = {
                // Issue #50: the host offers its journal-derived seed — the
                // REQUESTED concurrency (journaled at creation), falling back
                // to the planned chunk count for pre-#50 journals, clamped to
                // the engine ceiling. The create-time plan is
                // CHUNK_MIN-floored, so the pool alone under-seeds small
                // files. The adapter still OBEYS THE SERVER first (ADR-011):
                // no advertised extension → the classic lane, byte-for-byte.
                let events = self.j().events_for(job_id).map_err(EngineError::from)?;
                let asked = events
                    .iter()
                    .rev()
                    .find(|(_, k, _)| k == "job_created")
                    .and_then(|(_, _, p)| serde_json::from_str::<serde_json::Value>(p).ok())
                    .and_then(|v| v.get("max_conns").and_then(|c| c.as_u64()))
                    .map(|c| c as u32);
                let planned = self
                    .j()
                    .load_chunks(job_id)
                    .map_err(EngineError::from)?
                    .len() as u32;
                let seed = asked.unwrap_or(planned).clamp(1, MAX_WORKERS as u32);
                adapter
                    .probe_fleet(&spec, seed)
                    .await
                    .map_err(|e| EngineError::Adapter(e.to_string()))?
            };
            self.j()
                .append_event(
                    job_id,
                    "probe",
                    &serde_json::json!({
                        "resumable": probe.resumable,
                        "session_token": probe.session_token,
                        // Issue #13: the declaration channel is journaled with
                        // the probe so every future drive round (resume,
                        // crash recovery) re-derives the clamp from the
                        // journal — never from a live re-probe, which would
                        // orphan the session (ADR-002, #12).
                        "max_parallel": probe.max_parallel,
                        // Issue #44: the lane floor rides the same channel.
                        "min_chunk_bytes": probe.min_chunk_bytes,
                        // Issue #50: the OPTIONS-discovered capability,
                        // journaled so resume re-derives without re-probing
                        // (ADR-002). Absent in pre-#50 journals = false.
                        "concatenation": probe.concatenation,
                        // Issue #56: the PRESCRIBED uniform geometry, when
                        // the lane declares one — the reshape paths read it
                        // from the journal and never touch the shape.
                        "uniform_block": probe.uniform_block,
                    }),
                )
                .map_err(EngineError::from)?;
            if !probe.resumable {
                // One-shot endpoint: verify there is nothing to lose.
                let any_confirmed = self
                    .j()
                    .load_chunks(job_id)
                    .map_err(EngineError::from)?
                    .iter()
                    .any(|c| c.bytes_confirmed > 0);
                if any_confirmed {
                    record.state = self.transition(job_id, JobState::Unresumable).await?;
                    return Ok(record);
                }
            }
            // ADR-011 (issue #8): adapter declares, planner obeys. A cap that
            // binds re-plans the pool before the first worker runs — nothing
            // confirmed can be lost. One-shot endpoints are never re-planned:
            // chunking a stream that cannot resume would imply a capability
            // the protocol does not have (see `create_job`).
            if let Some(cap) = probe.max_chunk_bytes.filter(|_| probe.resumable) {
                self.apply_chunk_cap(job_id, cap)?;
            }
            // Issue #48 (M8 rung 1) + #57 (M9 rung 2): the link prober,
            // WIRED lane-wide. Opt-in via IUM_LINK_PROBE=1 — every byte is
            // sacred on a metered bearer, so the prober is a deliberate
            // instrument (the drill and the shell enable it explicitly).
            // Issue #57: the ADAPTER owns its wire, so the ADAPTER owns
            // its probe — the host asks the adapter it already holds
            // (ADR-011's shape, no adapter-name string matching). Lanes
            // without a shaped prober answer Ok(None) and stay
            // pre-#57 silent; a failed probe is a lost MEASUREMENT,
            // never a lost job — the skip is journaled and the job
            // proceeds exactly as before.
            if probe.resumable && Self::link_probe_requested() {
                match adapter.probe_link(&spec, 4, None).await {
                    Ok(Some(profile)) => {
                        self.j()
                            .append_event(
                                job_id,
                                crate::prober::LINK_PROFILE_EVENT,
                                &crate::prober::journal_payload(&profile),
                            )
                            .map_err(EngineError::from)?;
                        // The planner OBEYS the measurement — the same
                        // all-Pending journal-verified boundary as #8's cap
                        // replan, composed with the adapter's per-call cap
                        // (ADR-011 wins ties). The lane's own floor (S3's
                        // 5 MiB multipart minimum) applies inside the pool
                        // planner: the profile is DATA, not policy.
                        self.apply_measured_piece(
                            job_id,
                            profile.recommended_piece_bytes,
                            probe.max_chunk_bytes,
                        )?;
                    }
                    Ok(None) => {
                        // The lane has no shaped prober: pre-#57 silence.
                    }
                    Err(e) => {
                        let _ = self.j().append_event(
                            job_id,
                            "link_profile_skipped",
                            &serde_json::json!({ "reason": e.to_string() }),
                        );
                    }
                }
                // Issue #51 (M8 rung 4): the multi-path plan — the same
                // deliberate instrument as the link prober above (every
                // probe byte is sacred), driven by the operator's path
                // list (IUM_PATHS: comma-separated specs — "auto"
                // discovers from /sys/class/net, "addr:IP"/IP literal,
                // "iface:NAME"/bare name, "default"). Each declared path
                // is probed K=1 with its OWN canary bytes (ADR-009); a
                // path that cannot carry the canary is EXCLUDED and
                // journaled (`path_excluded`, 0 bps in the plan); the
                // weights are the plan the bonded lanes ride. A probe
                // failure is a lost MEASUREMENT, never a lost job — and
                // with no paths declared this block does not exist:
                // degradation IS today's behavior.
                // tus-shaped lanes only (#57 kept this gate): the path
                // prober speaks the tus wire (PATCH canaries), which is
                // exactly what a bonded tus lane rides.
                if probe.resumable && spec.adapter == "tus" {
                    let paths = Self::declared_paths();
                    if !paths.is_empty() {
                        let mut bps = Vec::with_capacity(paths.len());
                        for p in &paths {
                            match crate::prober::probe_path(
                                p,
                                std::path::Path::new(&spec.source_path),
                                spec.source_size,
                                &spec.endpoint,
                            )
                            .await
                            {
                                Ok(b) => bps.push(b),
                                Err(e) => {
                                    let _ = self.j().append_event(
                                        job_id,
                                        "path_excluded",
                                        &serde_json::json!({
                                            "path": p.name(),
                                            "why": e.to_string(),
                                        }),
                                    );
                                    bps.push(0.0);
                                }
                            }
                        }
                        let plan = crate::netpath::plan_weights(&paths, &bps)
                            .unwrap_or_else(|i| match i {});
                        self.j()
                            .append_event(
                                job_id,
                                "path_plan",
                                &crate::netpath::journal_payload(&plan, &bps),
                            )
                            .map_err(EngineError::from)?;
                    }
                }
            }
            // Tolerate a cancel that raced the probe: the journal is the
            // truth, and Canceled → Running is not a legal arrow.
            record.state = self
                .transition_or_canceled(job_id, JobState::Running)
                .await?;
        } else if record.state == JobState::Paused {
            // Resuming a paused job: journal Paused→Running before touching chunks.
            record.state = self
                .transition_or_canceled(job_id, JobState::Running)
                .await?;
        }
        // `Running` / `Finishing` on entry = crash recovery: the journal says
        // the job was mid-flight, so continue exactly there.

        if record.state == JobState::Running {
            // Issue #34: a journal write that fails under the fleet (ENOSPC)
            // parks the job honestly before the error surfaces.
            record.state = match self.drive_running(job_id, &spec, adapter.clone()).await {
                Ok(s) => s,
                Err(e) => return Err(self.park_on_journal_failure(job_id, e)),
            };
        }
        if record.state == JobState::Finishing {
            record.state = match self.finish(job_id, &spec, &adapter).await {
                Ok(s) => s,
                Err(e) => return Err(self.park_on_journal_failure(job_id, e)),
            };
        }
        self.require_job(job_id)
    }

    /// Consume `ProbeReport.max_chunk_bytes` (issue #8, ADR-011): when the
    /// declared per-call cap binds — some planned chunk exceeds it — re-plan
    /// with `n >= ceil(total/cap)` and journal the new plan (`replan_cap`).
    /// Runs at the probe boundary: every chunk must still be `Pending` with
    /// zero confirmed bytes, verified from the journal (ADR-002), not
    /// assumed. A cap that does not bind is a no-op: byte-identical
    /// behavior for adapters that never declare a binding ceiling.
    fn apply_chunk_cap(&self, job_id: &str, cap: u64) -> Result<()> {
        // Issue #56: a prescribed-geometry lane refuses EVERY reshape — the
        // wire (not a policy) dictates the shape, and the declaration was
        // journaled at probe. The cap is already honored by construction
        // (uniform_block == the wire's own block size).
        if self.declared_uniform_block(job_id).is_some() {
            return Ok(());
        }
        let chunks = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        let untouched = chunks
            .iter()
            .all(|c| c.state == ChunkState::Pending && c.bytes_confirmed == 0);
        let binds = chunks.iter().any(|c| c.end - c.begin > cap);
        if !untouched || !binds {
            return Ok(());
        }
        let before = chunks.len() as u32;
        let job = self.require_job(job_id)?;
        // Issue #44: the replan runs against the LANE floor, so a binding
        // cap on a small-floor lane (tus 64 KiB) yields lane-legal pieces
        // instead of fighting the S3-shaped default.
        let pool = ChunkPool::new_capped_floor(
            job.source_size,
            before,
            self.declared_lane_floor(job_id),
            Some(cap),
        );
        self.j()
            .replace_chunks(job_id, &pool.rows())
            .map_err(EngineError::from)?;
        self.j()
            .append_event(
                job_id,
                "replan_cap",
                &serde_json::json!({
                    "cap": cap,
                    "before": before,
                    "after": pool.chunks().len(),
                }),
            )
            .map_err(EngineError::from)?;
        Ok(())
    }

    /// Consume the prober's measured piece (issue #48, M8 rung 1): re-plan
    /// the pool so pieces match ~20 s of the MEASURED wire. Shrink AND grow
    /// bind here — this is the PLAN phase (nothing confirmed, nothing to
    /// lose); #27's grow rejection is about MID-FLIGHT resplits, a different
    /// regime with live leases and abandoned prefixes. Composed with the
    /// adapter's per-call cap (ADR-011 wins): the measured piece never
    /// overrides a tighter protocol ceiling. Same journal-verified
    /// all-Pending boundary as `apply_chunk_cap`; a measurement that agrees
    /// with the plan is a byte-identical no-op.
    fn apply_measured_piece(
        &self,
        job_id: &str,
        recommended: u64,
        adapter_cap: Option<u64>,
    ) -> Result<()> {
        let job = self.require_job(job_id)?;
        if job.source_size == 0 {
            return Ok(()); // the [(0,0)] plan has no piece to re-shape
        }
        // Issue #56: the measured piece cannot reshape a prescribed-geometry
        // lane — the uniform-tail grid IS the wire's arithmetic (ADR-011:
        // the adapter declaration wins over every host hint).
        if self.declared_uniform_block(job_id).is_some() {
            return Ok(());
        }
        let target = adapter_cap.map_or(recommended, |cap| recommended.min(cap));
        if target == 0 {
            return Ok(());
        }
        let chunks = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        let untouched = chunks
            .iter()
            .all(|c| c.state == ChunkState::Pending && c.bytes_confirmed == 0);
        if !untouched {
            return Ok(());
        }
        let before = chunks.len() as u64;
        let n_target = job.source_size.div_ceil(target);
        if n_target == before {
            return Ok(()); // the measurement agrees with the plan
        }
        let direction = if n_target < before { "grow" } else { "shrink" };
        let pool = ChunkPool::new_capped_floor(
            job.source_size,
            n_target.min(u32::MAX as u64) as u32,
            self.declared_lane_floor(job_id),
            None,
        );
        self.j()
            .replace_chunks(job_id, &pool.rows())
            .map_err(EngineError::from)?;
        self.j()
            .append_event(
                job_id,
                "replan_measured",
                &serde_json::json!({
                    "recommended_piece": recommended,
                    "applied_piece": target,
                    "direction": direction,
                    "before": before,
                    "after": pool.chunks().len(),
                }),
            )
            .map_err(EngineError::from)?;
        Ok(())
    }

    /// Consume `ProbeReport.max_parallel` (issue #13, ADR-011 channel):
    /// read the LAST journaled probe event for this job and return the
    /// adapter-declared concurrency ceiling, if any. The clamp is re-derived
    /// from the journal on EVERY drive round — resume and crash recovery
    /// included — so no drive round can exceed what the adapter declared,
    /// and none needs a live re-probe (which would orphan the session).
    /// Pre-#13 journals (events without the key) declare nothing: `None`.
    fn declared_max_parallel(&self, job_id: &str) -> Option<u32> {
        self.j()
            .events_for(job_id)
            .ok()?
            .into_iter()
            .rev()
            .find(|(_, kind, _)| kind == "probe")
            .and_then(|(_, _, payload)| serde_json::from_str::<serde_json::Value>(&payload).ok())
            .and_then(|v| v.get("max_parallel").cloned())
            .and_then(|v| v.as_u64().map(|n| n as u32))
    }

    /// Consume `ProbeReport.min_chunk_bytes` (issue #44, ADR-011 channel):
    /// read the LAST journaled probe event and return the adapter's LANE
    /// floor — the smallest piece this lane can legally confirm. Journals
    /// without the key (pre-#44) declare nothing and fall back to the
    /// engine default `CHUNK_MIN`, so old journals keep their exact plans.
    /// Clamped into `[1 KiB, CHUNK_MIN]`: sub-KiB pieces are header
    /// overhead with no feedback value, and a floor ABOVE the default would
    /// silently re-tighten lanes that never declared one.
    fn declared_lane_floor(&self, job_id: &str) -> u64 {
        self.j()
            .events_for(job_id)
            .ok()
            .and_then(|events| {
                events
                    .into_iter()
                    .rev()
                    .find(|(_, kind, _)| kind == "probe")
                    .map(|(_, _, payload)| payload)
            })
            .and_then(|payload| serde_json::from_str::<serde_json::Value>(&payload).ok())
            .and_then(|v| v.get("min_chunk_bytes").cloned())
            .and_then(|v| v.as_u64())
            .map(|floor| floor.clamp(1024, crate::chunk::CHUNK_MIN))
            .unwrap_or(crate::chunk::CHUNK_MIN)
    }

    /// Consume `ProbeReport.uniform_block` (issue #56, ADR-011 channel):
    /// read the LAST journaled probe event and return the lane's PRESCRIBED
    /// uniform chunk size, when it declared one. On such lanes the plan
    /// shape is the WIRE's arithmetic — resplit and cap replans are refused
    /// (they would break the protocol's geometry, Feishu 1062009) — and the
    /// shape survives resume/crash recovery unchanged (the journal re-derives,
    /// ADR-002). Journals without the key (pre-#56) declare nothing.
    fn declared_uniform_block(&self, job_id: &str) -> Option<u64> {
        self.j()
            .events_for(job_id)
            .ok()?
            .into_iter()
            .rev()
            .find(|(_, kind, _)| kind == "probe")
            .and_then(|(_, _, payload)| serde_json::from_str::<serde_json::Value>(&payload).ok())
            .and_then(|v| v.get("uniform_block").cloned())
            .and_then(|v| v.as_u64())
    }

    /// Consume the journaled link profile (issue #48, M8 rung 1): the LAST
    /// `link_profile` event's `recommended_fleet` — the prober's measurement
    /// of how many concurrent streams the link actually multiplies to. The
    /// seed composes at `AdaptiveFleet::seeded` time; live adaptation
    /// happens inside the adaptive value afterwards (#49).
    ///
    /// Issue #51: a journaled `path_plan` COMPOSES the seed — each live
    /// path is its own bearer (independently probed bps), so the system's
    /// stream count is per-path fleet × live paths. The single-link
    /// profile alone would understate a bonded lane (its measurement rode
    /// one bearer) and park the lane the bonding exists to un-park. The
    /// AIMD still governs live: a fake second bearer fails to double the
    /// progress and the adaptive value falls back on its own.
    fn measured_fleet(&self, job_id: &str) -> Option<u32> {
        let events = self.j().events_for(job_id).ok()?;
        let fleet = events
            .iter()
            .rev()
            .find(|(_, kind, _)| kind == crate::prober::LINK_PROFILE_EVENT)
            .and_then(|(_, _, payload)| serde_json::from_str::<serde_json::Value>(payload).ok())
            .and_then(|v| v.get("recommended_fleet").cloned())
            .and_then(|v| v.as_u64().map(|n| n as u32))?;
        let plan = events
            .iter()
            .rev()
            .find(|(_, kind, _)| kind == "path_plan")
            .and_then(|(_, _, payload)| serde_json::from_str::<serde_json::Value>(payload).ok());
        let live = plan
            .as_ref()
            .and_then(|p| p.get("paths").and_then(|v| v.as_array()))
            .map(|paths| {
                paths
                    .iter()
                    .filter(|p| !p.get("excluded").and_then(|e| e.as_bool()).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        Some(crate::netpath::compose_tus_partials(
            fleet,
            live.max(1) as u32,
        ))
    }

    /// Issue #50: run the adapter's finalize EXACTLY once per job —
    /// journal-before-execute (`concat_final` intent BEFORE the call, the
    /// `done` event with the produced URL AFTER; ADR-002). A journaled
    /// `done` makes every later call a no-op AND restores the final URL
    /// into a fresh process's adapter (a crash between `done` and the
    /// re-journaled probe event otherwise leaves the adapter blind to a
    /// materialized final — the journal is the truth, ADR-002). Classic
    /// lanes journal NOTHING: their finalize is a pure in-memory no-op.
    async fn concat_finalize_once(
        &self,
        job_id: &str,
        spec: &JobSpec,
        adapter: &dyn TransferAdapter,
    ) -> Result<()> {
        let events = self.j().events_for(job_id).map_err(EngineError::from)?;
        let last_probe = events
            .iter()
            .rev()
            .find(|(_, k, _)| k == "probe")
            .and_then(|(_, _, p)| serde_json::from_str::<serde_json::Value>(p).ok());
        // Concat topology = the journaled token is the JSON shape.
        let token_is_json = last_probe
            .as_ref()
            .and_then(|v| v.get("session_token").and_then(|t| t.as_str()))
            .map(|t| t.starts_with('{'))
            .unwrap_or(false);
        let last = events
            .iter()
            .rev()
            .find(|(_, k, _)| k == "concat_final")
            .and_then(|(_, _, p)| serde_json::from_str::<serde_json::Value>(p).ok());
        let done_url = last
            .as_ref()
            .filter(|v| v.get("state").and_then(|s| s.as_str()) == Some("done"))
            .and_then(|v| {
                v.get("final_url")
                    .and_then(|u| u.as_str())
                    .map(String::from)
            });
        if token_is_json {
            if let Some(url) = done_url {
                // Restore the complete session (final URL included) — the
                // crash-window repair, journaled truth over process memory.
                if let Some(tok) = last_probe
                    .as_ref()
                    .and_then(|v| v.get("session_token").and_then(|t| t.as_str()))
                {
                    if let Ok(mut t) = serde_json::from_str::<serde_json::Value>(tok) {
                        t["final"] = serde_json::json!(url);
                        adapter
                            .reattach(&spec.endpoint, &t.to_string())
                            .await
                            .map_err(|e| EngineError::Adapter(e.to_string()))?;
                    }
                }
                return Ok(());
            }
            self.j()
                .append_event(
                    job_id,
                    "concat_final",
                    &serde_json::json!({ "state": "intent" }),
                )
                .map_err(EngineError::from)?;
        }
        // Classic lanes (and only they) answer None — no journal noise.
        let url = adapter
            .finalize(spec)
            .await
            .map_err(|e| EngineError::Adapter(e.to_string()))?;
        if let (true, Some(url)) = (token_is_json, url) {
            self.j()
                .append_event(
                    job_id,
                    "concat_final",
                    &serde_json::json!({ "state": "done", "final_url": url }),
                )
                .map_err(EngineError::from)?;
            // Re-journal session truth (the repair path's pattern): the
            // newest probe event carries the final URL, so a resumed
            // process's reattach restores the complete concat session.
            if let Some(mut v) = last_probe {
                if let Some(tok) = v.get("session_token").and_then(|t| t.as_str()) {
                    if let Ok(mut t) = serde_json::from_str::<serde_json::Value>(tok) {
                        t["final"] = serde_json::json!(url);
                        v["session_token"] = serde_json::json!(t.to_string());
                        self.j()
                            .append_event(job_id, "probe", &v)
                            .map_err(EngineError::from)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Consume the LAST journaled fleet decision (issue #49): a fresh
    /// manager seeds its AIMD where the crashed one left off — the journal
    /// is the only truth (ADR-002), no process memory survives a restart.
    fn last_fleet_adjusted(&self, job_id: &str) -> Option<u32> {
        self.j()
            .events_for(job_id)
            .ok()?
            .into_iter()
            .rev()
            .find(|(_, kind, _)| kind == "fleet_adjusted")
            .and_then(|(_, _, payload)| serde_json::from_str::<serde_json::Value>(&payload).ok())
            .and_then(|v| v.get("to").cloned())
            .and_then(|v| v.as_u64().map(|n| n as u32))
    }

    /// The prober's opt-in switch (issue #48): `IUM_LINK_PROBE=1`. Every
    /// byte is sacred on a metered bearer — the prober is a deliberate
    /// instrument (the drill and the shell enable it explicitly), not
    /// ambient traffic.
    fn link_probe_requested() -> bool {
        std::env::var_os("IUM_LINK_PROBE")
            .map(|v| v == "1")
            .unwrap_or(false)
    }

    /// The operator's declared upload paths (issue #51): `IUM_PATHS`,
    /// comma-separated `netpath::parse_spec` entries — "auto" discovers
    /// from `/sys/class/net` (UP ifaces, loopback excluded). Empty or an
    /// unset variable = no bonding: the plan block never runs and the
    /// job proceeds exactly as before (the degradation rule).
    fn declared_paths() -> Vec<crate::netpath::NetPath> {
        let raw = std::env::var("IUM_PATHS").unwrap_or_default();
        let specs: Vec<&str> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if specs.is_empty() {
            return Vec::new();
        }
        if specs.iter().any(|s| s.eq_ignore_ascii_case("auto")) {
            return crate::netpath::discover(None);
        }
        specs
            .iter()
            .map(|s| crate::netpath::parse_spec(s))
            .collect()
    }

    /// The transfer loop (Sprint 1: concurrent workers over one shared pool).
    ///
    /// Each round: reload the pool from the journal (reclaiming leases left
    /// by a crashed driver), then run up to
    /// `min(pending, MAX_WORKERS, declared_max_parallel)` cooperative workers
    /// concurrently — the third bound is the adapter's probe declaration
    /// (issue #13), re-read from the journal each round. Every worker leases, streams its
    /// chunk with per-window progress sampling (slow-chunk eviction, spec
    /// §4.2), and journals its own confirms/errors. A fatal adapter error
    /// aborts the whole fleet; the driver journals `Failed` exactly once.
    async fn drive_running(
        &self,
        job_id: &str,
        spec: &JobSpec,
        adapter: Arc<dyn TransferAdapter>,
    ) -> Result<JobState> {
        let paused_rx = self.ensure_pause_flag(job_id).subscribe();
        let cancel_rx = self.ensure_cancel_flag(job_id).subscribe();
        let (abort_tx, abort_rx) = watch::channel(false);
        // Issue #27: the resplit epoch. The single winner bumps it BEFORE
        // reshaping the pool; peers drop their in-flight calls on the change.
        let (resplit_tx, resplit_rx) = watch::channel(0u64);
        let attempts: Arc<Mutex<HashMap<u32, u32>>> = Arc::new(Mutex::new(HashMap::new()));
        let live_bps: Arc<Mutex<HashMap<u32, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        let done_bps: Arc<Mutex<HashMap<u32, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        let best_live_bps = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let paused_seen = Arc::new(AtomicBool::new(false));
        // Issues #48+#49 (M8 rungs 1–2): the adaptive fleet. SEEDED from the
        // journal — the last decision, else the prober's measurement, else
        // the ceiling — clamped to what THIS job can use (a 3-piece job has
        // no business carrying a 32-wide waterline), and stepped once per
        // evidence window by the round's ticker. Resume-safe (ADR-002).
        let seed_ceiling = (MAX_WORKERS as u32).min(
            self.declared_max_parallel(job_id)
                .unwrap_or(MAX_WORKERS as u32)
                .max(1),
        );
        let initial_pending = self
            .load_pool(job_id)?
            .chunks()
            .iter()
            .filter(|c| c.state == ChunkState::Pending)
            .count() as u32;
        let aimd = Arc::new(Mutex::new(crate::fleet::AdaptiveFleet::seeded(
            self.last_fleet_adjusted(job_id)
                .or_else(|| self.measured_fleet(job_id))
                .unwrap_or(seed_ceiling)
                .min(seed_ceiling)
                .min(initial_pending.max(1)),
        )));

        let mut idle_reclaims: u32 = 0;
        loop {
            // Cancel is checked FIRST: if the journal says Canceled, the
            // journal is the truth — no new leases, no further transitions.
            if *cancel_rx.borrow() {
                return Ok(self.require_job(job_id)?.state);
            }
            if *paused_rx.borrow() {
                return self.transition_or_canceled(job_id, JobState::Paused).await;
            }

            // Fresh pool from the journal. Stale `Leased` rows left by a dead
            // driver return to `Pending` keeping their confirmed bytes.
            let pool = self.load_pool(job_id)?;
            let stale: Vec<(u32, u64, Option<String>)> = pool
                .chunks()
                .iter()
                .filter(|c| c.state == ChunkState::Leased)
                .map(|c| (c.idx, c.bytes_confirmed, c.adapter_token.clone()))
                .collect();
            if !stale.is_empty() {
                for (idx, confirmed, token) in stale {
                    self.j()
                        .update_chunk(
                            job_id,
                            idx,
                            ChunkState::Pending,
                            confirmed,
                            token.as_deref(),
                        )
                        .map_err(EngineError::from)?;
                }
                self.j()
                    .append_event(job_id, "leases_reclaimed", &serde_json::json!({}))
                    .map_err(EngineError::from)?;
                continue;
            }

            if pool.all_uploaded() {
                break;
            }
            let pending = pool
                .chunks()
                .iter()
                .filter(|c| c.state == ChunkState::Pending)
                .count();
            if pending == 0 {
                // Not all uploaded, nothing leasable, nothing reclaimed: an
                // inconsistent pool. The Finishing gate fails loudly instead
                // of declaring a lie — do not spin here. Issue #47 acceptance:
                // the exit leaves EVIDENCE first — the full pool snapshot
                // lands in the journal ahead of the loud verify gate.
                if !pool.all_uploaded() {
                    self.journal_pool_snapshot(job_id, "no_pending_no_leased")?;
                }
                break;
            }
            let before = pool.remaining_unconfirmed();

            // Issue #13 (ADR-011 channel): the adapter's declared concurrency
            // ceiling is a HARD bound on the fleet, re-read from the journal
            // probe event each round. A declared 0 is nonsense and clamps to
            // 1 — a violated floor costs requests, a violated limit fails
            // uploads, so a nonsensical declaration cannot spin the fleet.
            // Issue #49: the round spawns the full ceiling-worth of workers;
            // the LIVE waterline (`fleet_rx`, seeded from the measured
            // profile #48) parks the surplus at their lease boundaries and
            // grows them back — adaptation happens MID-round.
            let declared = self
                .declared_max_parallel(job_id)
                .unwrap_or(MAX_WORKERS as u32)
                .max(1) as usize;
            let ceiling = declared.min(MAX_WORKERS);
            let fleet = pending.min(ceiling).max(1);
            // Issue #49: the evidence window + the live waterline. The
            // ticker evaluates one window per tick and moves the waterline;
            // every decision lands in `changes` and is journaled by the
            // driver at round end (the ticker holds no journal handle — a
            // crash mid-round loses at most that round's decisions, and the
            // seed re-derives from the last durable one).
            let evidence: Arc<Mutex<Vec<(Instant, u64, bool)>>> = Arc::new(Mutex::new(Vec::new()));
            let (fleet_tx, fleet_rx) =
                watch::channel(aimd.lock().expect("aimd poisoned").current());
            let changes: Arc<Mutex<Vec<crate::fleet::FleetChange>>> =
                Arc::new(Mutex::new(Vec::new()));
            let ticker = {
                let evidence = evidence.clone();
                let changes = changes.clone();
                let fleet_tx = fleet_tx.clone();
                let aimd = aimd.clone();
                let ceiling_u32 = ceiling as u32;
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(EVIDENCE_WINDOW).await;
                        let entries: Vec<(Instant, u64, bool)> = {
                            let mut ev = evidence.lock().expect("evidence poisoned");
                            std::mem::take(&mut *ev)
                        };
                        let calls = entries.len() as u32;
                        let errors = entries.iter().filter(|(_, _, err)| *err).count() as u32;
                        let bytes: u64 = entries.iter().map(|(_, b, _)| *b).sum();
                        let mut aimd = aimd.lock().expect("aimd poisoned");
                        if let Some(change) = aimd.step(
                            bytes,
                            EVIDENCE_WINDOW.as_secs_f64(),
                            calls,
                            errors,
                            ceiling_u32,
                        ) {
                            let _ = fleet_tx.send(aimd.current());
                            changes.lock().expect("changes poisoned").push(change);
                        }
                    }
                })
            };

            let shared = DriveShared {
                pool: Arc::new(Mutex::new(pool)),
                attempts: attempts.clone(),
                live_bps: live_bps.clone(),
                done_bps: done_bps.clone(),
                best_live_bps: best_live_bps.clone(),
                abort_tx: abort_tx.clone(),
                abort_rx: abort_rx.clone(),
                failed: failed.clone(),
                paused_seen: paused_seen.clone(),
                first_lease_started: Arc::new(Mutex::new(None)),
                paused_rx: paused_rx.clone(),
                cancel_rx: cancel_rx.clone(),
                resplit_rx: resplit_rx.clone(),
                resplit_tx: resplit_tx.clone(),
                resplit_in_flight: Arc::new(AtomicBool::new(false)),
                fleet_rx: fleet_rx.clone(),
                evidence: evidence.clone(),
            };
            let futs: Vec<_> = (0..fleet)
                .map(|w| {
                    let worker_id = format!("w{w}");
                    // Borrow, don't move: every future shares one fleet state.
                    let shared: &DriveShared = &shared;
                    let adapter: &Arc<dyn TransferAdapter> = &adapter;
                    async move {
                        self.run_worker(shared, job_id, spec, adapter, &worker_id, w)
                            .await
                    }
                })
                .collect();
            let results = join_all(futs).await;
            // Issue #49: the ticker's watch term ends with the round.
            ticker.abort();

            if let Some(err) = results.into_iter().find_map(|r| r.err()) {
                return Err(err);
            }
            if failed.load(Ordering::SeqCst) {
                return self.transition(job_id, JobState::Failed).await;
            }
            if paused_seen.load(Ordering::SeqCst) {
                continue; // loop top journals Paused
            }
            // Issue #47: reclaim-then-decide. Every worker of the round has
            // EXITED — any `Leased` row in the journal is definitionally
            // ownerless within this driver (the round-start reclaim only
            // sees the state BEFORE the round ran). No exit to Finishing
            // while a reclaimable lease exists: return the rows to `Pending`
            // (keeping their confirmed bytes, the same rule the round-start
            // reclaim applies) and re-arm a fresh round. Bounded by
            // MAX_IDLE_RECLAIM_ROUNDS so a deterministic strand bug fails
            // loudly instead of livelocking.
            let end_pool = self.load_pool(job_id)?;
            // Issue #49: journal EVERY decision the ticker made this round
            // (ADR-002: the journal is the truth a fresh manager re-derives
            // the seed from). The worker-waterline already enforced each
            // decision live — this is the durable record.
            for change in changes.lock().expect("changes poisoned").drain(..) {
                self.j()
                    .append_event(
                        job_id,
                        "fleet_adjusted",
                        &serde_json::json!({
                            "from": change.from,
                            "to": change.to,
                            "reason": change.reason,
                            "goodput_bps": change.goodput_bps,
                        }),
                    )
                    .map_err(EngineError::from)?;
            }
            let stranded: Vec<(u32, u64, Option<String>)> = end_pool
                .chunks()
                .iter()
                .filter(|c| c.state == ChunkState::Leased)
                .map(|c| (c.idx, c.bytes_confirmed, c.adapter_token.clone()))
                .collect();
            if !stranded.is_empty() {
                if idle_reclaims >= MAX_IDLE_RECLAIM_ROUNDS {
                    self.journal_pool_snapshot(job_id, "round_end_reclaim_bound")?;
                    break;
                }
                idle_reclaims = idle_reclaims.saturating_add(1);
                for (idx, confirmed, token) in &stranded {
                    self.j()
                        .update_chunk(
                            job_id,
                            *idx,
                            ChunkState::Pending,
                            *confirmed,
                            token.as_deref(),
                        )
                        .map_err(EngineError::from)?;
                }
                self.j()
                    .append_event(
                        job_id,
                        "leases_reclaimed",
                        &serde_json::json!({
                            "when": "round_end",
                            "idxs": stranded.iter().map(|(idx, _, _)| idx).collect::<Vec<_>>(),
                        }),
                    )
                    .map_err(EngineError::from)?;
                continue;
            }
            // Spin guard: a round that moved nothing would loop forever.
            let after = end_pool.remaining_unconfirmed();
            if after == before {
                // Issue #47 acceptance: an exit that leaves non-uploaded
                // bytes must leave EVIDENCE — the full pool snapshot goes
                // to the journal ahead of the loud verify gate.
                if !end_pool.all_uploaded() {
                    self.journal_pool_snapshot(job_id, "spin_guard")?;
                }
                break;
            }
            idle_reclaims = 0;
        }
        // Cancel may have landed while the last round ran: the journal is
        // the truth — a Canceled job never enters Finishing.
        let state = self.require_job(job_id)?.state;
        if state == JobState::Canceled {
            return Ok(state);
        }
        self.transition(job_id, JobState::Finishing).await
    }

    /// Sleep that wakes early on abort/pause/cancel. Returns `false` when
    /// interrupted (the worker must exit without leasing anything).
    async fn nap(shared: &DriveShared, delay: Duration) -> bool {
        let mut abort_rx = shared.abort_rx.clone();
        let mut paused_rx = shared.paused_rx.clone();
        let mut cancel_rx = shared.cancel_rx.clone();
        tokio::select! {
            _ = tokio::time::sleep(delay) => true,
            _ = abort_rx.changed() => false,
            _ = cancel_rx.changed() => false,
            _ = paused_rx.changed() => {
                if *paused_rx.borrow() {
                    shared.paused_seen.store(true, Ordering::SeqCst);
                }
                false
            }
        }
    }

    /// One cooperative worker of a drive round. Leases from the shared pool,
    /// uploads one chunk at a time, journals every state change. Exits when
    /// the pool is exhausted, on pause, on fleet abort, or on a fatal error
    /// (after flagging the fleet). Issue #49: a worker ABOVE the live fleet
    /// waterline parks at this boundary — it neither exits (growth would
    /// want it back) nor leases (the shrink must bind) — and wakes on the
    /// waterline moving, the abort, the pause, or a bounded poll (which
    /// re-checks the pool: once it is exhausted the parked worker exits
    /// through the normal `lease() → None` path).
    async fn run_worker(
        &self,
        shared: &DriveShared,
        job_id: &str,
        spec: &JobSpec,
        adapter: &Arc<dyn TransferAdapter>,
        worker_id: &str,
        worker_index: usize,
    ) -> Result<()> {
        // watch receivers need `&mut` for `changed()`; every worker owns its
        // clone (watch is multi-consumer by design).
        let mut abort_rx = shared.abort_rx.clone();
        let mut resplit_rx = shared.resplit_rx.clone();
        let paused_rx = shared.paused_rx.clone();
        let mut fleet_rx = shared.fleet_rx.clone();
        loop {
            if *abort_rx.borrow() {
                return Ok(());
            }
            if *paused_rx.borrow() {
                shared.paused_seen.store(true, Ordering::SeqCst);
                return Ok(());
            }
            // Issue #49: over the waterline → park. A ≥ 1 waterline keeps
            // worker 0 alive, so SOMEONE always drains the pool; a parked
            // worker exits as soon as nothing leasable remains (all chunks
            // Done or held by under-waterline peers) — the poll bound makes
            // that exit bounded even without a waterline change.
            if worker_index as u32 >= *fleet_rx.borrow() {
                let nothing_leasable = shared
                    .pool
                    .lock()
                    .expect("pool mutex poisoned")
                    .chunks()
                    .iter()
                    .all(|c| c.state != ChunkState::Pending);
                if nothing_leasable {
                    return Ok(());
                }
                tokio::select! {
                    changed = fleet_rx.changed() => {
                        if changed.is_err() {
                            return Ok(()); // the round's sender is gone
                        }
                    }
                    _ = abort_rx.changed() => {}
                    _ = tokio::time::sleep(PARK_POLL) => {}
                }
                continue;
            }
            // Issue #47 (the stale-epoch poison): mark the CURRENT resplit
            // epoch as seen BEFORE leasing. The driver never consumes the
            // epoch change and a receiver clone preserves the unseen marker,
            // so after any resplit every lease taken in a later round died
            // instantly on its first `select!` poll — a STALE signal — and
            // stranded the just-journaled lease silently (the drill's piece
            // 0 died this way twice). A lease taken now is live in the
            // current plan; only a NEW bump (a real resplit under THIS
            // upload) may abort it.
            resplit_rx.borrow_and_update();

            // -- the global cap (#94): pace BETWEEN leases -----------------
            // Acquire the next lease's bytes BEFORE leasing: nothing is
            // held during the wait, so an abort/pause/resplit that lands
            // mid-wait simply re-runs THIS loop top (the same epoch sync,
            // the same flags — no stranded lease, no duplicate guard). The
            // wait also sits OUTSIDE the upload's eviction-tracked window,
            // so a capped wait can never be sampled as a stalled chunk —
            // zero false evictions by construction. The size is the pool's
            // current outstanding target (the largest unfinished part): the
            // eventual lease is that size or smaller (the file's tail), so
            // the pacing is honest-never-greedy. Unlimited = no-op.
            if self.rate().cap().is_some() {
                let want = shared
                    .pool
                    .lock()
                    .expect("pool mutex poisoned")
                    .outstanding_target();
                if want > 0 {
                    // One select! — `acquire` slices internally, so the wait
                    // is a single future; an abort or a resplit bump wins the
                    // biased race and `continue` re-runs the loop top (the
                    // flags' one home of meaning).
                    let paced_out = tokio::select! {
                        biased;
                        _ = abort_rx.changed() => Some(true),
                        _ = resplit_rx.changed() => Some(false),
                        _ = self.rate().acquire(want) => None,
                    };
                    if paced_out.is_some() {
                        continue; // the loop top owns every flag's meaning
                    }
                }
            }

            // -- lease: the in-memory pool decides, the journal mirrors it --
            // The pool lock is held across the journal mirror: a resplit
            // (issue #27) rewrites the whole chunks table by idx under that
            // same lock, so a lease and its durable mirror are atomic
            // against it — no "Leased" lie can land on the renumbered plan.
            let (lease, confirmed_at_lease) = {
                let mut pool = shared.pool.lock().expect("pool mutex poisoned");
                let lease = match pool.lease(worker_id) {
                    Some(l) => l,
                    // Every chunk is done or held by a peer; the driver
                    // re-arms a fresh round when the fleet thins out.
                    None => return Ok(()),
                };
                let confirmed = pool.chunks()[lease.idx as usize].bytes_confirmed;
                self.j()
                    .update_chunk(job_id, lease.idx, ChunkState::Leased, confirmed, None)
                    .map_err(EngineError::from)?;
                (lease, confirmed)
            };

            // Attempt number for THIS lease. Errors and evictions count
            // toward the retry budget; plain re-leases (pause/resume) do not.
            let attempt = {
                let attempts = shared.attempts.lock().expect("attempts poisoned");
                attempts.get(&lease.idx).copied().unwrap_or(0) + 1
            };

            // -- upload, with per-window progress sampling ------------------
            let progress = ProgressReporter::new();
            let lease_started = Instant::now();
            {
                // Issue #28: the starvation clock starts at the FIRST lease
                // of the job (set-once; later leases do not reset it).
                let mut first = shared
                    .first_lease_started
                    .lock()
                    .expect("first lease poisoned");
                if first.is_none() {
                    *first = Some(lease_started);
                }
            }
            let mut watch = ThroughputWatch::new(&self.eviction, Instant::now());
            let mut last_bps: Option<f64> = None;
            let mut last_median: Option<f64> = None;
            // Consecutive 423-lock waits (issue #44): counted across leases
            // while the drain persists, reset by ANY other outcome.
            let mut busy_waits: u32 = 0;
            let mut ticker =
                tokio::time::interval(self.eviction.window.max(Duration::from_millis(1)));
            ticker.tick().await; // Interval's first tick is immediate — consume it.

            enum Step {
                Confirmed(ChunkConfirm),
                Errored(AdapterError),
                Evicted,
                Aborted,
                /// Issue #27: the plan was reshaped under this lease — the
                /// lease identity is dead. Charge nothing (a resplit is not
                /// a failure) and re-lease from the new plan.
                ResplitUnderneath,
            }

            let upload = adapter.upload_chunk(spec, &lease, &progress);
            tokio::pin!(upload);
            let step = loop {
                tokio::select! {
                    biased;
                    _ = abort_rx.changed() => break Step::Aborted,
                    // Issue #27: the resplit winner bumped the epoch BEFORE
                    // reshaping the pool — drop this in-flight call (its
                    // lease was retired) instead of letting a stale
                    // confirm/error land on the renumbered plan.
                    _ = resplit_rx.changed() => break Step::ResplitUnderneath,
                    res = &mut upload => break match res {
                        Ok(confirm) => Step::Confirmed(confirm),
                        Err(err) => Step::Errored(err),
                    },
                    _ = ticker.tick() => {
                        let Some(bps) = watch.sample(Instant::now(), progress.moved()) else {
                            continue; // window not elapsed yet
                        };
                        let below = {
                            let mut live = shared.live_bps.lock().expect("live bps poisoned");
                            live.insert(lease.idx, bps);
                            // Issue #25: completed leases keep voting — the
                            // fleet's achieved rate is real knowledge even
                            // after its chunks finish.
                            let done = shared.done_bps.lock().expect("done bps poisoned");
                            let mut samples: Vec<f64> =
                                live.values().chain(done.values()).copied().collect();
                            let below = below_floor(bps, &mut samples, &self.eviction);
                            last_median = median(&mut samples);
                            below
                        };
                        watch.record_window(below);
                        last_bps = Some(bps);
                        // Track the fleet's best live window for vote capping
                        // (non-negative f64 bits order like the values).
                        shared
                            .best_live_bps
                            .fetch_max(bps.to_bits(), Ordering::Relaxed);
                        if watch.exhausted(&self.eviction) {
                            break Step::Evicted;
                        }
                        // Issue #27 (pattern #9): reshape the plan when the
                        // measured fleet rate says the outstanding parts are
                        // too coarse — and, since #28, when the fleet median
                        // is UNMEASURABLE (0) and the job has starved past
                        // slow_confirm: the frozen-link rule. Single-flight;
                        // the winner's own lease is retired with everyone
                        // else's.
                        let job_elapsed = shared
                            .first_lease_started
                            .lock()
                            .expect("first lease poisoned")
                            .map(|t| t.elapsed());
                        if self.execute_resplit(shared, job_id, last_median, job_elapsed) {
                            break Step::ResplitUnderneath;
                        }
                    }
                }
            };
            shared
                .live_bps
                .lock()
                .expect("live bps poisoned")
                .remove(&lease.idx);

            match step {
                Step::Confirmed(confirm) => {
                    #[allow(unused_assignments)] // control-flow barrier: the streak ends here
                    {
                        busy_waits = 0;
                    }
                    // Issue #25: the completed lease keeps ONE vote — its
                    // freshest measured rate (last window when one elapsed,
                    // else the lease mean) — so the eviction floor survives
                    // fast sub-window peers finishing around an impaired
                    // laggard. A confirm that moved nothing does not vote: a
                    // zero must not drag the floor down.
                    if progress.moved() > 0 {
                        let mean = progress.moved() as f64
                            / lease_started.elapsed().as_secs_f64().max(1e-9);
                        let vote = last_bps.unwrap_or(mean);
                        // Cap at the fleet's best live window: when a window
                        // has actually closed, no vote may outrun it — an
                        // un-capped sub-window mean inflates the median past
                        // every honest streamer (the integration drill
                        // caught exactly that on the first draft). When NO
                        // window ever sampled this round, the mean is the
                        // only evidence of the path's capability and stands
                        // uncapped — the lone-impaired-worker drill case.
                        let best = f64::from_bits(shared.best_live_bps.load(Ordering::Relaxed));
                        let vote = if best > 0.0 { vote.min(best) } else { vote };
                        shared
                            .done_bps
                            .lock()
                            .expect("done bps poisoned")
                            .insert(lease.idx, vote);
                    }
                    shared
                        .attempts
                        .lock()
                        .expect("attempts poisoned")
                        .remove(&lease.idx);
                    // Issue #27 guard: the plan may have been reshaped under
                    // this lease between the select! verdict and this arm —
                    // confirm.idx may now name a STRANGER chunk. Verify the
                    // row identity under the same pool lock the resplit
                    // winner holds, and drop a stale confirm honestly (its
                    // bytes are not part of the new plan; for S3 they are an
                    // orphaned server-side part the finish gate never lists).
                    let mut pool = shared.pool.lock().expect("pool mutex poisoned");
                    let stale = match pool.chunks().iter().find(|c| c.idx == confirm.idx) {
                        Some(c) => c.begin != lease.begin || c.end != lease.end,
                        None => true,
                    };
                    if stale {
                        drop(pool);
                        self.j()
                            .append_event(
                                job_id,
                                "stale_confirm_dropped",
                                &serde_json::json!({
                                    "idx": confirm.idx,
                                    "lease_begin": lease.begin,
                                    "lease_end": lease.end,
                                    "bytes": confirm.bytes_confirmed,
                                }),
                            )
                            .map_err(EngineError::from)?;
                        continue;
                    }
                    let (total, full) = self.apply_confirm(job_id, &confirm)?;
                    pool.report_confirmed(confirm.idx, total);
                    if full {
                        pool.complete(confirm.idx);
                    }
                    drop(pool);
                    // Issue #49: THIS call's server-confirmed bytes are the
                    // evidence window's goodput input.
                    shared.evidence.lock().expect("evidence poisoned").push((
                        Instant::now(),
                        confirm.bytes_confirmed,
                        false,
                    ));
                }
                Step::Errored(err) => {
                    // Issue #44 drill finding: tus 423 Locked is a WAIT, not a
                    // failure — the resource is serialized behind another
                    // writer or an abandoned request's server-side drain,
                    // which on a slow link outlives ANY retry ladder. Wait
                    // uncharged (the budget never sees it), outside the
                    // eviction's strike accounting (a fresh watch per lease
                    // never accumulates strikes across sub-second cycles),
                    // with exponential backoff capped at 8 s. Bounded by the
                    // user's cancel/pause, not by max_attempts.
                    if err.kind() == crate::retry::ErrorKind::Busy {
                        busy_waits = busy_waits.saturating_add(1);
                        let delay = Duration::from_secs(1 << busy_waits.saturating_sub(1).min(3));
                        self.j()
                            .append_event(
                                job_id,
                                "chunk_busy_wait",
                                &serde_json::json!({
                                    "idx": lease.idx,
                                    "wait_ms": delay.as_millis() as u64,
                                    "consecutive": busy_waits,
                                }),
                            )
                            .map_err(EngineError::from)?;
                        self.j()
                            .update_chunk(
                                job_id,
                                lease.idx,
                                ChunkState::Pending,
                                confirmed_at_lease,
                                None,
                            )
                            .map_err(EngineError::from)?;
                        {
                            let mut pool = shared.pool.lock().expect("pool mutex poisoned");
                            pool.evict(lease.idx, worker_id);
                        }
                        if !Self::nap(shared, delay).await {
                            return Ok(()); // abort/pause woke us early
                        }
                        continue;
                    }
                    #[allow(unused_assignments)] // control-flow barrier: the streak ends here
                    {
                        busy_waits = 0;
                    }
                    // Honor the server's own pacing when it declared one
                    // (429 + Retry-After) — spec §5.
                    let server_retry_after = match &err {
                        AdapterError::Throttled {
                            retry_after_ms: Some(ms),
                            ..
                        } => Some(Duration::from_millis(*ms)),
                        _ => None,
                    };
                    let action = self.policy.decide(err.kind(), attempt, server_retry_after);
                    // Issue #27 guard (same identity check as the confirm
                    // path): a stale error on a retired lease names a
                    // stranger idx — drop it honestly, charge no budget, and
                    // re-lease from the new plan.
                    let charge_budget = matches!(action, Action::Retry(_) | Action::Reattach);
                    {
                        let mut pool = shared.pool.lock().expect("pool mutex poisoned");
                        let stale = match pool.chunks().iter().find(|c| c.idx == lease.idx) {
                            Some(c) => c.begin != lease.begin || c.end != lease.end,
                            None => true,
                        };
                        if stale {
                            drop(pool);
                            self.j()
                                .append_event(
                                    job_id,
                                    "stale_error_dropped",
                                    &serde_json::json!({
                                        "idx": lease.idx,
                                        "lease_begin": lease.begin,
                                        "lease_end": lease.end,
                                        "kind": format!("{:?}", err.kind()),
                                    }),
                                )
                                .map_err(EngineError::from)?;
                            continue;
                        }
                        self.j()
                            .append_event(
                                job_id,
                                "chunk_error",
                                &serde_json::json!({
                                    "idx": lease.idx,
                                    "kind": format!("{:?}", err.kind()),
                                    "attempt": attempt,
                                    "action": format!("{action:?}"),
                                    // The reason rides the journal: a red run
                                    // must be self-diagnosing (the #39 lesson,
                                    // applied to the engine's own receipt). The
                                    // long-soak dispatch (issue #95 R3) burned a
                                    // round-trip on exactly this gap — 33
                                    // Transient errors with no text to name
                                    // their killer.
                                    "reason": err.to_string(),
                                }),
                            )
                            .map_err(EngineError::from)?;
                        // Back to the pool in the JOURNAL first, keeping
                        // whatever the server had confirmed before the error
                        // — the next lease resumes from there.
                        self.j()
                            .update_chunk(
                                job_id,
                                lease.idx,
                                ChunkState::Pending,
                                confirmed_at_lease,
                                None,
                            )
                            .map_err(EngineError::from)?;
                        // Issue #49: a COUNTED error is the AIMD error
                        // signal (Retry/Reattach charges only — Busy waits
                        // and stale drops are not errors; the fatal Fail
                        // path never reaches a window boundary).
                        if charge_budget {
                            shared.evidence.lock().expect("evidence poisoned").push((
                                Instant::now(),
                                0,
                                true,
                            ));
                        }
                        pool.evict(lease.idx, worker_id);
                        // Retry-budget charge under the same lock: a resplit
                        // clears this map when it rebuilds the plan; a stale
                        // insert racing it could resurrect a dead budget.
                        if charge_budget {
                            shared
                                .attempts
                                .lock()
                                .expect("attempts poisoned")
                                .insert(lease.idx, attempt);
                        }
                    }
                    match action {
                        Action::Retry(delay) => {
                            tracing::warn!(job = job_id, idx = lease.idx, ?delay, "retrying chunk");
                            if !Self::nap(shared, delay).await {
                                return Ok(()); // abort/pause woke us early
                            }
                        }
                        Action::Reattach => {
                            // Issue #12: the taxonomy promises session
                            // recovery. Replay the journaled probe token into
                            // the adapter (never re-probe — that would
                            // orphan the live session), count the attempt,
                            // and continue: the chunk re-leases from its
                            // confirmed offset. A session that is really
                            // gone re-errors on the next lease and drains
                            // this same budget into `Fail`.
                            self.reattach_from_journal(
                                job_id,
                                spec,
                                adapter.as_ref(),
                                "mid_transfer",
                            )
                            .await?;
                        }
                        Action::Reauthenticate | Action::Fail => {
                            tracing::warn!(job = job_id, idx = lease.idx, "chunk failed for good");
                            shared.failed.store(true, Ordering::SeqCst);
                            let _ = shared.abort_tx.send(true);
                            return Ok(());
                        }
                    }
                }
                Step::Evicted => {
                    // Issue #27 guard: an eviction decided just before a
                    // resplit landed names a retired lease — drop honestly,
                    // charge nothing, re-lease from the new plan.
                    {
                        let pool = shared.pool.lock().expect("pool mutex poisoned");
                        let stale = match pool.chunks().iter().find(|c| c.idx == lease.idx) {
                            Some(c) => c.begin != lease.begin || c.end != lease.end,
                            None => true,
                        };
                        if stale {
                            drop(pool);
                            self.j()
                                .append_event(
                                    job_id,
                                    "stale_error_dropped",
                                    &serde_json::json!({
                                        "idx": lease.idx,
                                        "lease_begin": lease.begin,
                                        "lease_end": lease.end,
                                        "kind": "eviction",
                                    }),
                                )
                                .map_err(EngineError::from)?;
                            continue;
                        }
                    }
                    // Slow-chunk eviction (spec §4.2): in-flight bytes were
                    // never server-confirmed, so the chunk returns to the
                    // pool keeping exactly its lease-time confirmation.
                    // Evictions count toward the retry budget — a chunk that
                    // is evicted forever must fail deterministically, not spin.
                    self.j()
                        .update_chunk(
                            job_id,
                            lease.idx,
                            ChunkState::Pending,
                            confirmed_at_lease,
                            None,
                        )
                        .map_err(EngineError::from)?;
                    shared
                        .attempts
                        .lock()
                        .expect("attempts poisoned")
                        .insert(lease.idx, attempt);
                    if attempt > self.policy.max_attempts {
                        self.j()
                            .append_event(
                                job_id,
                                "chunk_evict_exhausted",
                                &serde_json::json!({ "idx": lease.idx, "attempt": attempt }),
                            )
                            .map_err(EngineError::from)?;
                        shared.failed.store(true, Ordering::SeqCst);
                        let _ = shared.abort_tx.send(true);
                        return Ok(());
                    }
                    self.j()
                        .append_event(
                            job_id,
                            "chunk_evicted",
                            &serde_json::json!({
                                "idx": lease.idx,
                                "bps": last_bps,
                                "median_bps": last_median,
                                "strikes": watch.strikes(),
                                "attempt": attempt,
                            }),
                        )
                        .map_err(EngineError::from)?;
                    shared
                        .pool
                        .lock()
                        .expect("pool mutex poisoned")
                        .evict(lease.idx, worker_id);
                    // loop: this or a peer worker re-leases and resumes from
                    // the confirmed offset.
                }
                Step::ResplitUnderneath => {
                    // Issue #27: the plan was reshaped under this lease.
                    // Issue #47: "there is nothing to journal" is true ONLY
                    // for leases the winner's rewrite retired. A lease taken
                    // AFTER the rewrite is LIVE in the current plan — and
                    // before the loop-top epoch sync, every post-resplit
                    // lease was one. Same identity guard as the confirm/
                    // error/eviction arms: a live lease returns to the pool
                    // (journal first, then the in-memory evict, under the
                    // same pool lock the resplit rewrite holds — the lease
                    // block's atomicity invariant); a retired identity stays
                    // unjournaled (the rewrite already reset its row). Charge
                    // no retry budget either way (a resplit is not a failure;
                    // the winner cleared the budgets with the rebuild).
                    let mut pool = shared.pool.lock().expect("pool mutex poisoned");
                    let live = match pool.chunks().iter().find(|c| c.idx == lease.idx) {
                        Some(c) => c.begin == lease.begin && c.end == lease.end,
                        None => false,
                    };
                    if live {
                        self.j()
                            .update_chunk(
                                job_id,
                                lease.idx,
                                ChunkState::Pending,
                                confirmed_at_lease,
                                None,
                            )
                            .map_err(EngineError::from)?;
                        pool.evict(lease.idx, worker_id);
                    }
                }
                Step::Aborted => return Ok(()),
            }
        }
    }

    /// Issue #27: consult the resplit policy and, when it fires and this
    /// call wins the single-flight gate, reshape the pending plan from the
    /// measured fleet rate: retire leases (peers cancel via the epoch bump
    /// BEFORE the mutation), split/merge pending chunks around the new
    /// target, persist the whole plan transactionally, journal the decision
    /// with its real inputs, and restart the retry budgets (the plan they
    /// keyed on no longer exists). Returns `true` when a resplit executed —
    /// the caller's own lease is retired and it must re-lease.
    fn execute_resplit(
        &self,
        shared: &DriveShared,
        job_id: &str,
        median_bps: Option<f64>,
        job_elapsed: Option<Duration>,
    ) -> bool {
        // Issue #56: no resplit on prescribed-geometry lanes — the shape is
        // the wire's arithmetic, and a resplit would produce parts the
        // server rejects (1062009). The candidate is never even computed.
        if self.declared_uniform_block(job_id).is_some() {
            return false;
        }
        let candidate = {
            let pool = shared.pool.lock().expect("pool mutex poisoned");
            let current = pool.outstanding_target();
            // Issue #44: the floor regime runs against the LANE floor (the
            // adapter's journaled declaration), not the S3-shaped global
            // constant — on a 64 KiB-floor lane the resplit can actually
            // reach wire-sized pieces.
            let lane_floor = self.declared_lane_floor(job_id);
            match median_bps {
                Some(m) if m > 0.0 => self.resplit.plan(Some(m), current, lane_floor),
                _ => {
                    // Issue #28 frozen-link rule: an unmeasurable fleet median
                    // (every sample zero — the adapter's writes never complete
                    // into buffers this small) gives plan() nothing to act on;
                    // a job starving past slow_confirm with an oversized part
                    // still outstanding IS the signal. The floor is the only
                    // honest target when no rate can be measured.
                    let starved = job_elapsed.is_some_and(|e| e >= self.resplit.slow_confirm);
                    if starved {
                        self.resplit.plan_frozen(current, lane_floor)
                    } else {
                        None
                    }
                }
            }
        };
        let Some(new_target) = candidate else {
            return false;
        };
        // Single-flight: one reshaper per round; every other proposer just
        // re-checks on its next window.
        if shared
            .resplit_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        // Cancel peers FIRST (they drop their in-flight calls on the epoch
        // change), then reshape under the pool lock — the same lock every
        // journal-by-idx path holds, so no stale write can interleave.
        shared.resplit_tx.send_modify(|e| *e += 1);
        let mut pool = shared.pool.lock().expect("pool mutex poisoned");
        let chunks_before = pool.chunks().len();
        pool.resplit_pending(new_target, self.declared_lane_floor(job_id));
        let rows = pool.rows();
        let chunks_after = rows.len();
        let journaled = (|| -> std::result::Result<(), EngineError> {
            self.j()
                .replace_chunks(job_id, &rows)
                .map_err(EngineError::from)?;
            self.j()
                .append_event(
                    job_id,
                    "chunk_resplit",
                    &serde_json::json!({
                        "target": new_target,
                        "median_bps": median_bps.unwrap_or(0.0),
                        "chunks_before": chunks_before,
                        "chunks_after": chunks_after,
                    }),
                )
                .map_err(EngineError::from)?;
            Ok(())
        })();
        drop(pool);
        // The plan identity was rebuilt: retry budgets restart on it.
        // Charging old-idx attempts onto the renumbered plan would exhaust
        // budgets for bytes never retried.
        shared.attempts.lock().expect("attempts poisoned").clear();
        shared.resplit_in_flight.store(false, Ordering::Release);
        match journaled {
            Ok(()) => true,
            Err(err) => {
                // The journal write failed: the in-memory pool is reshaped
                // but NOT the durable truth — never run diverged from disk.
                // Fail the job loudly (the driver journals Failed).
                tracing::error!(job = job_id, "resplit journal write failed: {err}");
                shared.failed.store(true, Ordering::SeqCst);
                let _ = shared.abort_tx.send(true);
                false
            }
        }
    }

    /// Verify-before-declare gate (pattern #10).
    ///
    /// Issue #24: when verify REFUSES on a resumable job (the server's part
    /// list cannot assemble — e.g. SeaweedFS keeps a duplicate entry for a
    /// completed-but-unacknowledged PUT that the resume re-sent), the gate
    /// makes ONE counted, journaled repair: re-probe (fresh multipart
    /// session, journaled per ADR-002), reset every chunk to Pending over
    /// the SAME spans, re-drive, re-verify. Verify-before-declare is not
    /// weakened — the Complete only ever fires after a CLEAN reconcile on
    /// the fresh session. One attempt: the `verify_repair` marker makes a
    /// second refusal terminal (Failed, honest).
    async fn finish(
        &self,
        job_id: &str,
        spec: &JobSpec,
        adapter: &Arc<dyn TransferAdapter>,
    ) -> Result<JobState> {
        // The repair marker: ONE `verify_repair` event per job, ever. Read
        // up front so a journal that already carries one (a previous drive's
        // repair) cannot repair again.
        let mut repair_used = self
            .j()
            .events_for(job_id)
            .map_err(EngineError::from)?
            .iter()
            .any(|(_, k, _)| k == "verify_repair");
        // Issue #26: a cancel that lands around the gate is honored HERE —
        // before any verify/Complete I/O — so a canceled job never completes
        // the multipart and the object never exists. cancel() journaled
        // Canceled journal-first; the gate just stands down. The journal is
        // the authority (ADR-002), not the flag.
        if self.require_job(job_id)?.state == JobState::Canceled {
            return Ok(JobState::Canceled);
        }
        let ok_detail: Option<String> = loop {
            // Issue #12, hole 2: reconcile the adapter's session state FROM
            // THE JOURNAL before verifying — a fresh process's adapter map
            // is empty, and verify must not die one step before Done.
            // No-op without a journaled probe token (one-shot endpoints,
            // 0-byte S3 sources).
            self.reattach_from_journal(job_id, spec, adapter.as_ref(), "finishing_entry")
                .await?;
            // Issue #50: materialize the concatenation final BEFORE verify
            // gates on it — the final URL only exists once finalize ran.
            self.concat_finalize_once(job_id, spec, adapter.as_ref())
                .await?;
            let verify = adapter
                .verify(spec)
                .await
                .map_err(|e| EngineError::Adapter(e.to_string()))?;
            if verify.ok {
                break verify.detail;
            }
            self.j()
                .append_event(
                    job_id,
                    "verify_failed",
                    &serde_json::json!({ "detail": verify.detail }),
                )
                .map_err(EngineError::from)?;
            let record = self.require_job(job_id)?;
            if repair_used || record.capability_badge != "resumable" {
                return self.transition(job_id, JobState::Failed).await;
            }
            // -- the one repair -------------------------------------------------
            repair_used = true;
            self.j()
                .append_event(
                    job_id,
                    "verify_repair",
                    &serde_json::json!({ "detail": verify.detail }),
                )
                .map_err(EngineError::from)?;
            // Fresh multipart session: a NEW CreateMultipartUpload, journaled
            // as the newest probe event (session truth, ADR-002; the #13
            // clamp re-derivation reads the LAST probe and stays correct).
            let probe = adapter
                .probe(spec)
                .await
                .map_err(|e| EngineError::Adapter(e.to_string()))?;
            self.j()
                .append_event(
                    job_id,
                    "probe",
                    &serde_json::json!({
                        "resumable": probe.resumable,
                        "session_token": probe.session_token,
                        "max_parallel": probe.max_parallel,
                        "min_chunk_bytes": probe.min_chunk_bytes,
                        "uniform_block": probe.uniform_block,
                    }),
                )
                .map_err(EngineError::from)?;
            // Reset every chunk to Pending over the SAME spans — journal
            // truth, nothing invented: the plan is byte-identical, only the
            // per-chunk progress is zeroed because the fresh session holds
            // nothing.
            let rows: Vec<crate::journal::ChunkRow> = self
                .j()
                .load_chunks(job_id)
                .map_err(EngineError::from)?
                .iter()
                .map(|c| crate::journal::ChunkRow {
                    idx: c.idx,
                    begin: c.begin,
                    end: c.end,
                    state: crate::chunk::ChunkState::Pending,
                    bytes_confirmed: 0,
                    adapter_token: None,
                })
                .collect();
            self.j()
                .replace_chunks(job_id, &rows)
                .map_err(EngineError::from)?;
            self.transition(job_id, JobState::Running).await?;
            let driven = self
                .drive_running(job_id, spec, Arc::clone(adapter))
                .await?;
            if driven != JobState::Finishing {
                // Paused / Canceled / Failed during the repair re-drive: the
                // journal holds the truth, the state is terminal-or-paused.
                return Ok(driven);
            }
        };

        // Persist every uploaded chunk as verified (journal is the truth).
        let pool = self.load_pool(job_id)?;
        // Consistency gate: reaching Finishing with an incomplete chunk means
        // the supervisor lied somewhere upstream — fail instead of declaring.
        let incomplete = pool.chunks().iter().any(|c| {
            !matches!(c.state, ChunkState::Uploaded | ChunkState::Verified)
                || c.bytes_confirmed != c.len()
        });
        if incomplete {
            self.j()
                .append_event(
                    job_id,
                    "verify_inconsistent",
                    &serde_json::json!({
                        "detail": "reached Finishing with chunks not fully confirmed",
                        "chunks": [pool.chunks().iter().map(|c| serde_json::json!({
                            "idx": c.idx, "state": c.state, "confirmed": c.bytes_confirmed, "len": c.len()
                        })).collect::<Vec<_>>()],
                    }),
                )
                .map_err(EngineError::from)?;
            return self.transition(job_id, JobState::Failed).await;
        }
        for c in pool
            .chunks()
            .iter()
            .filter(|c| c.state == ChunkState::Uploaded)
        {
            self.j()
                .update_chunk(
                    job_id,
                    c.idx,
                    ChunkState::Verified,
                    c.len(),
                    c.adapter_token.as_deref(),
                )
                .map_err(EngineError::from)?;
        }
        self.j()
            .append_event(
                job_id,
                "verified",
                &serde_json::json!({ "detail": ok_detail }),
            )
            .map_err(EngineError::from)?;
        // Issue #26 razor tolerance: a cancel that raced the verify call
        // journaled Canceled underneath the gate while the server already
        // completed the object (verify fired Complete). The engine never
        // declares a false Done: the journal keeps the user's Canceled and
        // this event records the server truth.
        match self.transition(job_id, JobState::Done).await {
            Ok(s) => Ok(s),
            Err(EngineError::IllegalTransition(ref e)) if e.from == JobState::Canceled => {
                self.j()
                    .append_event(
                        job_id,
                        "complete_after_cancel",
                        &serde_json::json!({ "detail": ok_detail }),
                    )
                    .map_err(EngineError::from)?;
                Ok(JobState::Canceled)
            }
            Err(e) => Err(e),
        }
    }

    /// Replay the job's LAST journaled `probe` event's `session_token` into
    /// the adapter (issue #12). The journal is the only session truth
    /// (ADR-002): mid-transfer recovery and fresh-process Finishing entry
    /// both reattach from it — never by re-probing, which would orphan the
    /// live session (an S3 re-probe opens a NEW multipart upload). Returns
    /// `Ok(false)` when there is nothing to replay (no probe event or no
    /// token — one-shot endpoints, 0-byte S3 sources).
    async fn reattach_from_journal(
        &self,
        job_id: &str,
        spec: &JobSpec,
        adapter: &dyn TransferAdapter,
        source: &str,
    ) -> Result<bool> {
        let token = self
            .j()
            .events_for(job_id)
            .map_err(EngineError::from)?
            .into_iter()
            .rev()
            .find(|(_, kind, _)| kind == "probe")
            .and_then(|(_, _, payload)| {
                serde_json::from_str::<serde_json::Value>(&payload)
                    .ok()?
                    .get("session_token")?
                    .as_str()
                    .map(str::to_string)
            })
            .filter(|t| !t.is_empty());
        let Some(token) = token else {
            return Ok(false);
        };
        adapter
            .reattach(&spec.endpoint, &token)
            .await
            .map_err(|e| EngineError::Adapter(format!("reattach failed: {e}")))?;
        self.j()
            .append_event(job_id, "reattach", &serde_json::json!({ "source": source }))
            .map_err(EngineError::from)?;
        Ok(true)
    }

    fn load_pool(&self, job_id: &str) -> Result<ChunkPool> {
        let record = self.require_job(job_id)?;
        let rows = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        Ok(ChunkPool::from_rows(record.source_size, rows))
    }

    /// Issue #47 acceptance: every drive-loop exit that leaves non-uploaded
    /// bytes journals the FULL pool snapshot first — the loud verify gate
    /// then fails with evidence, and the journal alone explains the shape
    /// (which row, which state, how many confirmed bytes) without a live
    /// process to interrogate. Per-chunk shape matches the verify gate's
    /// `verify_inconsistent` payload.
    fn journal_pool_snapshot(&self, job_id: &str, why: &str) -> Result<()> {
        let pool = self.load_pool(job_id)?;
        let chunks: Vec<serde_json::Value> = pool
            .chunks()
            .iter()
            .map(|c| {
                serde_json::json!({
                    "idx": c.idx, "state": c.state,
                    "confirmed": c.bytes_confirmed, "len": c.len(),
                })
            })
            .collect();
        self.j()
            .append_event(
                job_id,
                "drive_pool_snapshot",
                &serde_json::json!({ "why": why, "chunks": [chunks] }),
            )
            .map_err(EngineError::from)?;
        Ok(())
    }

    /// Apply a server acknowledgment. `confirm.bytes_confirmed` is
    /// SEGMENT-relative (what THIS call got acked for `[resume_from, end)`);
    /// the manager composes it with the chunk's prior confirmations into the
    /// absolute in-chunk total. Full confirmation completes the chunk; a
    /// partial one returns it to `Pending` KEEPING its confirmed bytes, so
    /// the next lease resumes at `resume_from` instead of restarting (and a
    /// partial chunk can never be declared done by accident).
    ///
    /// Returns `(absolute in-chunk confirmed total, full)` so a concurrent
    /// caller can mirror the state into the in-memory pool.
    fn apply_confirm(&self, job_id: &str, confirm: &ChunkConfirm) -> Result<(u64, bool)> {
        let rows = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        let row = rows.iter().find(|r| r.idx == confirm.idx).ok_or_else(|| {
            EngineError::Adapter(format!("confirm for unknown chunk {}", confirm.idx))
        })?;
        let chunk_len = row.end - row.begin;
        let confirmed = row
            .bytes_confirmed
            .saturating_add(confirm.bytes_confirmed)
            .min(chunk_len);
        let full = confirmed == chunk_len;

        self.j()
            .update_chunk(
                job_id,
                confirm.idx,
                if full {
                    ChunkState::Uploaded
                } else {
                    ChunkState::Pending
                },
                confirmed,
                confirm.adapter_token.as_deref(),
            )
            .map_err(EngineError::from)?;
        self.j()
            .append_event(
                job_id,
                "chunk_confirmed",
                &serde_json::json!({
                    "idx": confirm.idx,
                    "bytes_confirmed": confirmed,
                    "full": full,
                }),
            )
            .map_err(EngineError::from)?;
        tracing::debug!(job = job_id, idx = confirm.idx, confirmed, "chunk confirm");
        Ok((confirmed, full))
    }
}
