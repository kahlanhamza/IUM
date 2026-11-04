//! Verify-repair contract (issue #24): when the ADR-010 verify gate refuses
//! a resumed upload because the SERVER's part list cannot assemble (the
//! SeaweedFS completed-but-unacknowledged PUT leaves a duplicate part entry
//! behind a re-PUT), the finish gate must make ONE counted, journaled
//! repair attempt — re-probe (fresh multipart), re-drive every chunk span
//! from journal truth, re-verify — instead of ending an otherwise-perfect
//! upload as Failed. Verify-before-declare is not weakened: the Complete
//! still fires only after a CLEAN reconcile on the fresh session.
//!
//! * repair happy path: resumed job, verify fails once → Done, with
//!   `verify_failed` + `verify_repair` + a SECOND probe event journaled and
//!   every chunk span re-sent exactly once.
//! * repair is bounded: a job whose verify keeps failing ends Failed with
//!   exactly ONE `verify_repair` (no spin).
//! * repair never runs twice: the second pass has no marker, so a second
//!   failure is terminal.

#![allow(dead_code)] // test-support helpers shared across fixtures

mod common;

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

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
        adapter: "repair_mock".into(),
        endpoint: "mock://repair".into(),
    }
}

/// Session-honest mock (issue #12 semantics): `has_session` starts false;
/// probe creates a session (token counter); verify without a session is
/// SessionLost. Verify can be armed to fail N times (reconcile-shape
/// failure — the #24 wire truth), and every upload span is counted so the
/// repair's full re-send is observable.
struct RepairMock {
    cap: Capability,
    has_session: std::sync::atomic::AtomicBool,
    session_seq: AtomicU32,
    fail_verify_remaining: AtomicU32,
    verify_calls: AtomicU32,
    wire: AtomicU64,
    upload_calls: AtomicU32,
    reattach_count: AtomicU32,
    probe_calls: AtomicU32,
}

impl RepairMock {
    fn new(cap: Capability, fail_verify_times: u32) -> Arc<Self> {
        Arc::new(Self {
            cap,
            has_session: std::sync::atomic::AtomicBool::new(false),
            session_seq: AtomicU32::new(0),
            fail_verify_remaining: AtomicU32::new(fail_verify_times),
            verify_calls: AtomicU32::new(0),
            wire: AtomicU64::new(0),
            upload_calls: AtomicU32::new(0),
            reattach_count: AtomicU32::new(0),
            probe_calls: AtomicU32::new(0),
        })
    }
    fn wire(&self) -> u64 {
        self.wire.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl TransferAdapter for RepairMock {
    fn name(&self) -> &'static str {
        "repair_mock"
    }
    fn capabilities(&self) -> Capability {
        self.cap
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        let n = self.session_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.has_session
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(ProbeReport {
            resumable: true,
            session_token: Some(format!("tok-{n}")),
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
        })
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        lease: &Lease,
        _progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        if !self.has_session.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AdapterError::SessionLost(
                "no session for endpoint; restore from the journaled probe event".into(),
            ));
        }
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        let span = lease.end - lease.begin;
        self.wire.fetch_add(span, Ordering::SeqCst);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: Some(format!("N:etag-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        if !self.has_session.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AdapterError::SessionLost(
                "verify with an empty session map".into(),
            ));
        }
        let call = self.verify_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let remaining = self.fail_verify_remaining.fetch_sub(1, Ordering::SeqCst);
        if remaining > 0 {
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "reconcile failed, upload left incomplete: part numbers not contiguous \
                     (verify call #{call})"
                )),
            });
        }
        Ok(VerifyReport {
            ok: true,
            detail: None,
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        self.reattach_count.fetch_add(1, Ordering::SeqCst);
        self.has_session
            .store(true, std::sync::atomic::Ordering::SeqCst);
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
async fn repair_recovers_a_resumed_job_when_verify_refuses_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "repair.bin", 8 * MIB);
    let db = dir.path().join("repair.db");

    // Session 1: real probe + full upload, then rewind to the crash window:
    // every chunk Uploaded with full bytes, job Finishing — a kill between
    // the last confirm and the verify (the #12 fixture shape).
    {
        let journal = SessionJournal::open(&db).unwrap();
        let mut m = JobManager::with_journal(journal).unwrap();
        m.register_adapter(RepairMock::new(Capability::new(Capability::RESUMABLE), 0));
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

    // Session 2: FRESH manager + FRESH adapter (session map empty), verify
    // armed to refuse ONCE with the #24 reconcile detail. Desired: the
    // finish gate repairs ONCE (re-probe → re-drive → re-verify) → Done.
    let mock = RepairMock::new(Capability::new(Capability::RESUMABLE), 1);
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::clone(&mock) as Arc<dyn TransferAdapter>);

    let job = m.list_jobs().unwrap().remove(0);
    assert_eq!(job.state, ium_core::state::JobState::Finishing);
    let out = m.run_job(&job.id).await.unwrap();

    assert_eq!(
        out.state,
        ium_core::state::JobState::Done,
        "one verify refusal on a resumed job must be repaired, not terminal"
    );
    let kinds = event_kinds(&m, &job.id);
    assert!(
        kinds.iter().filter(|k| *k == "verify_failed").count() >= 1,
        "the refusal is journaled: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|k| *k == "verify_repair").count(),
        1,
        "exactly one journaled repair: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|k| *k == "probe").count(),
        2,
        "the repair re-probes (fresh multipart session, journaled): {kinds:?}"
    );
    // The repair re-sends every span: session 1 pushed 8 MiB, the repair
    // pushed 8 MiB again (the fresh multipart holds nothing).
    assert_eq!(
        mock.wire(),
        8 * MIB,
        "the repair re-drive re-uploads the whole span set onto the fresh session"
    );
    // Session 1's probe belongs to the first manager's adapter instance;
    // THIS instance probes exactly once — the repair's fresh session.
    assert_eq!(mock.probe_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn repair_is_bounded_one_attempt_then_honest_failed() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "stuck.bin", 6 * MIB);
    let db = dir.path().join("stuck.db");

    {
        let journal = SessionJournal::open(&db).unwrap();
        let mut m = JobManager::with_journal(journal).unwrap();
        m.register_adapter(RepairMock::new(Capability::new(Capability::RESUMABLE), 0));
        let rec = m.create_job(&spec_for(&path, 6 * MIB), 2).unwrap();
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

    // Verify refuses FOREVER: one repair, then honest Failed — no spin.
    let mock = RepairMock::new(Capability::new(Capability::RESUMABLE), u32::MAX);
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::clone(&mock) as Arc<dyn TransferAdapter>);

    let job = m.list_jobs().unwrap().remove(0);
    let out = m.run_job(&job.id).await.unwrap();
    assert_eq!(
        out.state,
        ium_core::state::JobState::Failed,
        "a verify that never passes must exhaust the single repair into Failed"
    );
    let kinds = event_kinds(&m, &job.id);
    assert_eq!(
        kinds.iter().filter(|k| *k == "verify_repair").count(),
        1,
        "repair is a one-shot: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|k| *k == "probe").count(),
        2,
        "exactly one repair re-probe: {kinds:?}"
    );
}

#[tokio::test]
async fn verify_passing_first_try_never_repairs() {
    // The normal path must stay byte-identical: no verify_repair event, one
    // probe, Done.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "clean.bin", 4 * MIB);
    let journal = SessionJournal::open(dir.path().join("clean.db")).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    let mock = RepairMock::new(Capability::new(Capability::RESUMABLE), 0);
    m.register_adapter(Arc::clone(&mock) as Arc<dyn TransferAdapter>);
    let rec = m.create_job(&spec_for(&path, 4 * MIB), 2).unwrap();
    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(out.state, ium_core::state::JobState::Done);
    let kinds = event_kinds(&m, &rec.id);
    assert_eq!(kinds.iter().filter(|k| *k == "verify_repair").count(), 0);
    assert_eq!(kinds.iter().filter(|k| *k == "probe").count(), 1);
    assert_eq!(mock.wire(), 4 * MIB);
}
