//! Cancel-during-verify semantics (issue #26): the Finishing gate is REAL
//! network I/O on a KB/s uplink (reattach → ListParts reconcile →
//! CompleteMultipartUpload), so a user's Cancel must be honored there.
//! Contract points:
//!
//! * `Finishing → Canceled` is a legal transition (the user wins; a Canceled
//!   terminal never claims Done);
//! * the razor window — the cancel raced the verify call and verify already
//!   fired the server-side Complete — NEVER declares a false Done: the
//!   journal keeps the user's `Canceled` and a `complete_after_cancel` event
//!   records the server truth.
//!
//! Verify-before-declare is untouched: Done is only ever journaled from a
//! clean gate.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::{JobManager, JobSpec, Lease, SessionJournal};

const MIB: u64 = 1024 * 1024;

fn make_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    let mut remaining = size;
    let mut seed = 0x9E3779B97F4A7C15u64;
    while remaining > 0 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let chunk = seed.to_le_bytes();
        let n = remaining.min(8) as usize;
        f.write_all(&chunk[..n]).unwrap();
        remaining -= n as u64;
    }
    path
}

fn spec_for(path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "cancel_mock".into(),
        endpoint: "mock://cancel".into(),
    }
}

/// Session-honest mock. With the hook UNARMED it is a plain always-succeeds
/// adapter; ARMED, its verify() triggers the user's Cancel WHILE the gate's
/// verify call is in flight — the razor window of issue #26.
struct CancelInVerifyMock {
    cap: Capability,
    has_session: AtomicBool,
    session_seq: AtomicU32,
    verify_calls: AtomicU32,
    hook: Mutex<Option<(Arc<JobManager>, String)>>,
}

impl CancelInVerifyMock {
    fn new(cap: Capability) -> Arc<Self> {
        Arc::new(Self {
            cap,
            has_session: AtomicBool::new(false),
            session_seq: AtomicU32::new(0),
            verify_calls: AtomicU32::new(0),
            hook: Mutex::new(None),
        })
    }
}

#[async_trait::async_trait]
impl TransferAdapter for CancelInVerifyMock {
    fn name(&self) -> &'static str {
        "cancel_mock"
    }
    fn capabilities(&self) -> Capability {
        self.cap
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        let n = self.session_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.has_session.store(true, Ordering::SeqCst);
        Ok(ProbeReport {
            resumable: true,
            session_token: Some(format!("tok-{n}")),
            concatenation: false,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
            uniform_block: None,
        })
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        lease: &Lease,
        _progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        if !self.has_session.load(Ordering::SeqCst) {
            return Err(AdapterError::SessionLost(
                "no session for endpoint; restore from the journaled probe event".into(),
            ));
        }
        let span = lease.end - lease.begin;
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: Some(format!("N:etag-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        if !self.has_session.load(Ordering::SeqCst) {
            return Err(AdapterError::SessionLost(
                "verify with an empty session map".into(),
            ));
        }
        self.verify_calls.fetch_add(1, Ordering::SeqCst);
        // The razor window: the user's Cancel lands WHILE the gate's verify
        // is in flight (cancel() journals Canceled journal-first).
        let hook = self.hook.lock().unwrap().clone();
        if let Some((mgr, id)) = hook {
            mgr.cancel(&id)
                .await
                .map_err(|e| AdapterError::Unknown(format!("cancel during verify: {e}")))?;
        }
        Ok(VerifyReport {
            ok: true,
            detail: Some("mock holds all bytes; the server completed".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        self.has_session.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn event_kinds(m: &JobManager, job: &str) -> Vec<String> {
    m.snapshot(job)
        .unwrap()
        .events
        .into_iter()
        .map(|(_, k, _)| k)
        .collect()
}

#[tokio::test]
async fn cancel_racing_verify_never_declares_done() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "cancel.bin", 8 * MIB);
    let db = dir.path().join("cancel.db");

    // Session 1: a clean upload (hook unarmed = plain adapter), then rewind
    // to the crash window — job Finishing, every chunk Uploaded with full
    // bytes (the #12/#24 fixture shape): the gate is about to verify.
    {
        let journal = SessionJournal::open(&db).unwrap();
        let mut m = JobManager::with_journal(journal).unwrap();
        m.register_adapter(CancelInVerifyMock::new(Capability::new(
            Capability::RESUMABLE,
        )));
        let rec = m.create_job(&spec_for(&path, 8 * MIB), 2).unwrap();
        m.run_job(&rec.id).await.unwrap();
        m.journal()
            .set_job_state(&rec.id, ium_core::state::JobState::Finishing)
            .unwrap();
        for c in m.snapshot(&rec.id).unwrap().chunks {
            m.journal()
                .update_chunk(
                    &rec.id,
                    c.idx,
                    ium_core::chunk::ChunkState::Uploaded,
                    c.end - c.begin,
                    c.adapter_token.as_deref(),
                )
                .unwrap();
        }
        m.journal().checkpoint_wal().unwrap();
    }

    // Session 2: fresh manager; the mock's verify() cancels the job
    // MID-VERIFY (the razor window) and then reports a clean reconcile.
    let mock = CancelInVerifyMock::new(Capability::new(Capability::RESUMABLE));
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::clone(&mock) as Arc<dyn TransferAdapter>);
    let m = Arc::new(m);
    let job = m.list_jobs().unwrap().remove(0);
    assert_eq!(job.state, ium_core::state::JobState::Finishing);
    *mock.hook.lock().unwrap() = Some((Arc::clone(&m), job.id.clone()));

    let out = m.run_job(&job.id).await.unwrap();

    // The user's Cancel wins the terminal race — and the engine NEVER
    // declares a false Done even though verify reported a clean reconcile.
    assert_eq!(
        out.state,
        ium_core::state::JobState::Canceled,
        "the cancel that raced verify owns the terminal state"
    );
    let kinds = event_kinds(&m, &job.id);
    assert_eq!(
        kinds
            .iter()
            .filter(|k| *k == "complete_after_cancel")
            .count(),
        1,
        "the server truth (verify fired Complete) is recorded once: {kinds:?}"
    );
    let final_state = m.snapshot(&job.id).unwrap().job.state;
    assert_eq!(
        final_state,
        ium_core::state::JobState::Canceled,
        "the journal never ends Done for a canceled job"
    );
    assert_eq!(mock.verify_calls.load(Ordering::SeqCst), 1);

    // Break the Arc cycle the razor-window hook forms (Manager → adapter →
    // Manager): the mock outlives the run holding the manager that holds
    // the mock, so without this neither ever drops. The sanitizer lane
    // (issue #95 R4, run 42542779782: 1332 bytes / 10 objects, all
    // indirect) caught exactly this — the first find of the ASan program.
    // A production manager-callback API must be Weak-based; this hook is
    // test scaffolding, and the test owns breaking it.
    *mock.hook.lock().unwrap() = None;
}
