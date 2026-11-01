//! JobManager — the orchestrator facade (pattern #5: supervised engine,
//! in-process). Every state transition is journaled before it executes;
//! `bytes_confirmed` only ever carries server acknowledgments.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::adapter::{AdapterRegistry, ChunkConfirm, TransferAdapter};
use crate::chunk::{ChunkPool, ChunkState};
use crate::journal::{ChunkRow, JobRecord, SessionJournal};
use crate::retry::{Action, RetryPolicy};
use crate::state::JobState;
use crate::{EngineError, JobSpec, Result};

/// UI-facing snapshot of a job (spec §8 transparency).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobSnapshot {
    pub job: JobRecord,
    pub chunks: Vec<ChunkRow>,
    pub events: Vec<(i64, String, String)>,
}

pub struct JobManager {
    journal: Mutex<SessionJournal>,
    registry: AdapterRegistry,
    policy: RetryPolicy,
    /// job_id -> pause flag (watch keeps the last value; workers observe it).
    pause_flags: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl JobManager {
    pub fn open(journal_path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            journal: Mutex::new(SessionJournal::open(journal_path).map_err(EngineError::from)?),
            registry: AdapterRegistry::new(),
            policy: RetryPolicy::default(),
            pause_flags: Mutex::new(HashMap::new()),
        })
    }

    pub fn with_journal(journal: SessionJournal) -> Result<Self> {
        Ok(Self {
            journal: Mutex::new(journal),
            registry: AdapterRegistry::new(),
            policy: RetryPolicy::default(),
            pause_flags: Mutex::new(HashMap::new()),
        })
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn TransferAdapter>) {
        self.registry.register(adapter);
    }

    pub fn set_policy(&mut self, policy: RetryPolicy) {
        self.policy = policy;
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
    pub fn create_job(&self, spec: &JobSpec, max_conns: u32) -> Result<JobRecord> {
        let adapter = self.adapter(spec)?;
        let cap = adapter.capabilities();
        let id = crate::ids::new_id();
        let pool = ChunkPool::new(spec.source_size, max_conns);
        let record = self
            .j()
            .insert_job(
                &id,
                &spec.source_path,
                spec.source_size,
                &spec.adapter,
                &spec.endpoint,
                cap.badge(),
                &pool.rows(),
            )
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

    async fn transition(&self, job_id: &str, from: JobState, to: JobState) -> Result<JobState> {
        from.transition(to).map_err(EngineError::from)?;
        // Journal BEFORE acting (design rule #2).
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
    /// the journal, so only unconfirmed work is attempted.
    pub async fn run_job(&self, job_id: &str, data: Arc<Vec<u8>>) -> Result<JobRecord> {
        let mut record = self.require_job(job_id)?;
        let spec = JobSpec {
            source_path: record.source_path.clone(),
            source_size: record.source_size,
            adapter: record.adapter.clone(),
            endpoint: record.endpoint.clone(),
        };
        let adapter = self.adapter(&spec)?;

        // -- Probing ------------------------------------------------------
        if record.state == JobState::Created {
            record.state = self
                .transition(job_id, record.state, JobState::Probing)
                .await?;
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
                let chunks = self.j().load_chunks(job_id).map_err(EngineError::from)?;
                let any_confirmed = chunks.iter().any(|c| c.bytes_confirmed > 0);
                if any_confirmed {
                    let to = self
                        .transition(job_id, record.state, JobState::Unresumable)
                        .await?;
                    record.state = to;
                    return Ok(record);
                }
            }
            let to = self
                .transition(job_id, record.state, JobState::Running)
                .await?;
            record.state = to;
        }

        // Resuming a paused job: journal Paused→Running before touching chunks.
        if record.state == JobState::Paused {
            record.state = self
                .transition(job_id, JobState::Paused, JobState::Running)
                .await?;
        }

        let paused_rx = self.ensure_pause_flag(job_id).subscribe();

        // -- Running ------------------------------------------------------
        loop {
            if *paused_rx.borrow() {
                let to = self
                    .transition(job_id, JobState::Running, JobState::Paused)
                    .await?;
                record.state = to;
                return Ok(record);
            }

            let mut pool = self.load_pool(job_id)?;
            if pool.all_uploaded() {
                break;
            }

            let Some(lease) = pool.lease("main") else {
                break; // nothing leasable (all uploaded/pending-by-others)
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

            match adapter.upload_chunk(&spec, &lease, data.clone()).await {
                Ok(confirm) => {
                    self.apply_confirm(job_id, &mut pool, &confirm)?;
                }
                Err(err) => {
                    let action = self.policy.decide(err.kind(), lease.attempt + 1, None);
                    self.j().append_event(job_id, "chunk_error", &serde_json::json!({
                            "idx": lease.idx, "kind": format!("{:?}", err.kind()), "action": format!("{action:?}"),
                        }))
                        .map_err(EngineError::from)?;
                    match action {
                        Action::Retry(delay) => {
                            pool.evict(lease.idx, "main");
                            tracing::warn!(job = job_id, idx = lease.idx, ?delay, "retrying chunk");
                            tokio::time::sleep(delay).await;
                        }
                        Action::Reauthenticate | Action::Reattach | Action::Fail => {
                            pool.evict(lease.idx, "main");
                            self.j()
                                .replace_chunks(job_id, &pool.rows())
                                .map_err(EngineError::from)?;
                            let to = self
                                .transition(job_id, JobState::Running, JobState::Failed)
                                .await?;
                            record.state = to;
                            return Ok(record);
                        }
                    }
                }
            }
        }

        // -- Finishing (verify-before-declare, pattern #10) ----------------
        let to = self
            .transition(job_id, JobState::Running, JobState::Finishing)
            .await?;
        record.state = to;
        let verify = adapter
            .verify(&spec)
            .await
            .map_err(|e| EngineError::Adapter(e.to_string()))?;

        if verify.ok {
            // persist all uploaded chunks as verified
            let mut pool = self.load_pool(job_id)?;
            let to_verify: Vec<(u32, u64, Option<String>)> = pool
                .chunks()
                .iter()
                .filter(|c| c.state == ChunkState::Uploaded)
                .map(|c| (c.idx, c.len(), c.adapter_token.clone()))
                .collect();
            for (idx, len, token) in to_verify {
                pool.mark_verified(idx);
                self.j()
                    .update_chunk(job_id, idx, ChunkState::Verified, len, token.as_deref())
                    .map_err(EngineError::from)?;
            }
            self.j()
                .append_event(
                    job_id,
                    "verified",
                    &serde_json::json!({ "detail": verify.detail }),
                )
                .map_err(EngineError::from)?;
            let to = self
                .transition(job_id, JobState::Finishing, JobState::Done)
                .await?;
            record.state = to;
        } else {
            self.j()
                .append_event(
                    job_id,
                    "verify_failed",
                    &serde_json::json!({ "detail": verify.detail }),
                )
                .map_err(EngineError::from)?;
            let to = self
                .transition(job_id, JobState::Finishing, JobState::Failed)
                .await?;
            record.state = to;
        }
        Ok(record)
    }

    fn load_pool(&self, job_id: &str) -> Result<ChunkPool> {
        let record = self.require_job(job_id)?;
        let rows = self.j().load_chunks(job_id).map_err(EngineError::from)?;
        Ok(ChunkPool::from_rows(record.source_size, rows))
    }

    fn apply_confirm(
        &self,
        job_id: &str,
        pool: &mut ChunkPool,
        confirm: &ChunkConfirm,
    ) -> Result<()> {
        let chunk_len = pool.chunks()[confirm.idx as usize].len();
        let confirmed = confirm.bytes_confirmed.min(chunk_len);
        let new_state = if confirmed == chunk_len {
            ChunkState::Uploaded
        } else {
            ChunkState::Leased
        };
        pool.report_confirmed(confirm.idx, confirmed);
        if confirmed == chunk_len {
            pool.complete(confirm.idx);
        }
        self.j()
            .update_chunk(
                job_id,
                confirm.idx,
                new_state,
                confirmed,
                confirm.adapter_token.as_deref(),
            )
            .map_err(EngineError::from)?;
        self.j()
            .append_event(
                job_id,
                "chunk_confirmed",
                &serde_json::json!({
                    "idx": confirm.idx, "bytes_confirmed": confirmed,
                }),
            )
            .map_err(EngineError::from)?;
        Ok(())
    }
}
