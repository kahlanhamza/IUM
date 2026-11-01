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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::adapter::{AdapterRegistry, ChunkConfirm, TransferAdapter};
use crate::chunk::{ChunkPool, ChunkState};
use crate::journal::{JobRecord, SessionJournal};
use crate::retry::{Action, RetryPolicy};
use crate::state::JobState;
use crate::{EngineError, JobSpec, Result};

/// UI-facing snapshot of a job (spec §8 transparency).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobSnapshot {
    pub job: JobRecord,
    pub chunks: Vec<crate::journal::ChunkRow>,
    pub events: Vec<(i64, String, String)>,
}

pub struct JobManager {
    journal: Mutex<SessionJournal>,
    registry: AdapterRegistry,
    policy: RetryPolicy,
    /// job_id -> pause flag (watch keeps the last value; workers observe it).
    pause_flags: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// job ids with an active driver task — enforces single-driver.
    active: Mutex<HashSet<String>>,
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

impl JobManager {
    pub fn open(journal_path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            journal: Mutex::new(SessionJournal::open(journal_path).map_err(EngineError::from)?),
            registry: AdapterRegistry::new(),
            policy: RetryPolicy::default(),
            pause_flags: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
        })
    }

    pub fn with_journal(journal: SessionJournal) -> Result<Self> {
        Ok(Self {
            journal: Mutex::new(journal),
            registry: AdapterRegistry::new(),
            policy: RetryPolicy::default(),
            pause_flags: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
        })
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn TransferAdapter>) {
        self.registry.register(adapter);
    }

    pub fn set_policy(&mut self, policy: RetryPolicy) {
        self.policy = policy;
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
        let pool = ChunkPool::new(spec.source_size, workers);
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

    pub fn resume_signal(&self, job_id: &str) -> Result<()> {
        self.require_job(job_id)?;
        if let Some(tx) = self.pause_flags.lock().unwrap().get(job_id) {
            tx.send_replace(false);
        }
        Ok(())
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
            .set_job_state(job_id, to)
            .map_err(EngineError::from)?;
        self.j()
            .append_event(
                job_id,
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
        if matches!(record.state, JobState::Done | JobState::Unresumable) {
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
            let probe = adapter
                .probe(&spec)
                .await
                .map_err(|e| EngineError::Adapter(e.to_string()))?;
            self.j()
                .append_event(
                    job_id,
                    "probe",
                    &serde_json::json!({
                        "resumable": probe.resumable,
                        "session_token": probe.session_token,
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
            record.state = self.transition(job_id, JobState::Running).await?;
        } else if record.state == JobState::Paused {
            // Resuming a paused job: journal Paused→Running before touching chunks.
            record.state = self.transition(job_id, JobState::Running).await?;
        }
        // `Running` / `Finishing` on entry = crash recovery: the journal says
        // the job was mid-flight, so continue exactly there.

        if record.state == JobState::Running {
            record.state = self.drive_running(job_id, &spec, adapter.as_ref()).await?;
        }
        if record.state == JobState::Finishing {
            record.state = self.finish(job_id, &spec, adapter.as_ref()).await?;
        }
        self.require_job(job_id)
    }

    /// The transfer loop. Leases one chunk at a time (Sprint 0 supervisor;
    /// the concurrent worker pool lands in Sprint 1 on the same pool API).
    async fn drive_running(
        &self,
        job_id: &str,
        spec: &JobSpec,
        adapter: &dyn TransferAdapter,
    ) -> Result<JobState> {
        let paused_rx = self.ensure_pause_flag(job_id).subscribe();
        // Per-chunk attempt counter for THIS drive. Persisted per-chunk attempt
        // columns arrive with the Sprint-1 resplit work; the journal keeps the
        // full error history via `chunk_error` events either way.
        let mut attempts: HashMap<u32, u32> = HashMap::new();

        loop {
            if *paused_rx.borrow() {
                return self.transition(job_id, JobState::Paused).await;
            }

            let mut pool = self.load_pool(job_id)?;
            if pool.all_uploaded() {
                break;
            }
            let Some(lease) = pool.lease("main") else {
                // Nothing leasable but not all uploaded: a crashed driver left
                // `Leased` rows in the journal. Reclaim them (confirmed bytes
                // are kept) and lease again.
                let mut reclaimed = false;
                for c in pool
                    .chunks()
                    .iter()
                    .filter(|c| c.state == ChunkState::Leased)
                {
                    self.j()
                        .update_chunk(
                            job_id,
                            c.idx,
                            ChunkState::Pending,
                            c.bytes_confirmed,
                            c.adapter_token.as_deref(),
                        )
                        .map_err(EngineError::from)?;
                    reclaimed = true;
                }
                if reclaimed {
                    continue;
                }
                break; // nothing leasable and nothing stuck
            };
            self.j()
                .update_chunk(
                    job_id,
                    lease.idx,
                    ChunkState::Leased,
                    pool.chunks()[lease.idx as usize].bytes_confirmed,
                    None,
                )
                .map_err(EngineError::from)?;

            let attempt = attempts.get(&lease.idx).copied().unwrap_or(0) + 1;
            match adapter.upload_chunk(spec, &lease).await {
                Ok(confirm) => {
                    attempts.remove(&lease.idx);
                    self.apply_confirm(job_id, &confirm)?;
                }
                Err(err) => {
                    // Honor the server's own pacing when it declared one
                    // (429 + Retry-After) — spec §5, pattern: FDM's
                    // Retry-After vocabulary.
                    let server_retry_after = match &err {
                        crate::adapter::AdapterError::Throttled {
                            retry_after_ms: Some(ms),
                            ..
                        } => Some(std::time::Duration::from_millis(*ms)),
                        _ => None,
                    };
                    let action = self.policy.decide(err.kind(), attempt, server_retry_after);
                    self.j()
                        .append_event(
                            job_id,
                            "chunk_error",
                            &serde_json::json!({
                                "idx": lease.idx,
                                "kind": format!("{:?}", err.kind()),
                                "attempt": attempt,
                                "action": format!("{action:?}"),
                            }),
                        )
                        .map_err(EngineError::from)?;
                    // The chunk goes back to the pool in the JOURNAL first,
                    // keeping whatever the server had confirmed before the
                    // error — the next lease resumes from there.
                    self.j()
                        .update_chunk(
                            job_id,
                            lease.idx,
                            ChunkState::Pending,
                            pool.chunks()[lease.idx as usize].bytes_confirmed,
                            None,
                        )
                        .map_err(EngineError::from)?;
                    match action {
                        Action::Retry(delay) => {
                            attempts.insert(lease.idx, attempt);
                            tracing::warn!(job = job_id, idx = lease.idx, ?delay, "retrying chunk");
                            tokio::time::sleep(delay).await;
                        }
                        Action::Reauthenticate | Action::Reattach | Action::Fail => {
                            tracing::warn!(job = job_id, idx = lease.idx, "chunk failed for good");
                            return self.transition(job_id, JobState::Failed).await;
                        }
                    }
                }
            }
        }
        self.transition(job_id, JobState::Finishing).await
    }

    /// Verify-before-declare gate (pattern #10).
    async fn finish(
        &self,
        job_id: &str,
        spec: &JobSpec,
        adapter: &dyn TransferAdapter,
    ) -> Result<JobState> {
        let verify = adapter
            .verify(spec)
            .await
            .map_err(|e| EngineError::Adapter(e.to_string()))?;

        if !verify.ok {
            self.j()
                .append_event(
                    job_id,
                    "verify_failed",
                    &serde_json::json!({ "detail": verify.detail }),
                )
                .map_err(EngineError::from)?;
            return self.transition(job_id, JobState::Failed).await;
        }

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
                &serde_json::json!({ "detail": verify.detail }),
            )
            .map_err(EngineError::from)?;
        self.transition(job_id, JobState::Done).await
    }

    fn load_pool(&self, job_id: &str) -> Result<ChunkPool> {
        let record = self.require_job(job_id)?;
        let rows = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        Ok(ChunkPool::from_rows(record.source_size, rows))
    }

    /// Apply a server acknowledgment. `confirm.bytes_confirmed` is
    /// SEGMENT-relative (what THIS call got acked for `[resume_from, end)`);
    /// the manager composes it with the chunk's prior confirmations into the
    /// absolute in-chunk total. Full confirmation completes the chunk; a
    /// partial one returns it to `Pending` KEEPING its confirmed bytes, so
    /// the next lease resumes at `resume_from` instead of restarting (and a
    /// partial chunk can never be declared done by accident).
    fn apply_confirm(&self, job_id: &str, confirm: &ChunkConfirm) -> Result<()> {
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
        Ok(())
    }
}
