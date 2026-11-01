//! End-to-end engine drills on the mock adapter — the skeleton of the
//! Sprint-1 chaos harness: run-to-done, resume-from-journal, pause,
//! failure classification.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, TransferAdapter, VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;

/// Adapter that confirms every chunk on first attempt; optionally fails the
/// first N upload calls to exercise the retry path, and can slow each chunk
/// down so other tasks can interact mid-run.
struct MockAdapter {
    cap: Capability,
    fail_first: AtomicU32,
    chunk_delay_ms: u64,
}

#[async_trait::async_trait]
impl TransferAdapter for MockAdapter {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn capabilities(&self) -> Capability {
        self.cap
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        // Probe honesty mirrors the declared capability (spec §5).
        Ok(ProbeReport {
            resumable: self.cap.has(Capability::RESUMABLE),
            max_chunk_bytes: None,
            session_token: Some("mock-session".into()),
        })
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        lease: &Lease,
        _data: Arc<Vec<u8>>,
    ) -> Result<ChunkConfirm, AdapterError> {
        if self.chunk_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.chunk_delay_ms)).await;
        }
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(AdapterError::Transient("mock network blip".into()));
        }
        let full = lease.end - lease.resume_from;
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: full,
            adapter_token: Some(format!("tok-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("mock hash ok".into()),
        })
    }
}

fn spec(size: u64) -> JobSpec {
    JobSpec {
        source_path: "/tmp/ium-test.bin".into(),
        source_size: size,
        adapter: "mock".into(),
        endpoint: "mock://endpoint".into(),
    }
}

fn manager_with(cap: Capability) -> JobManager {
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(MockAdapter {
        cap,
        fail_first: AtomicU32::new(0),
        chunk_delay_ms: 0,
    }));
    m
}

#[tokio::test]
async fn job_runs_to_done_with_all_chunks_verified() {
    let m = manager_with(Capability::new(
        Capability::RESUMABLE | Capability::TAIL_RACE | Capability::DYNAMIC_CHUNKS,
    ));
    let rec = m.create_job(&spec(20 * MIB), 4).unwrap();
    assert_eq!(rec.state, JobState::Created);
    assert_eq!(rec.capability_badge, "resumable");

    let done = m.run_job(&rec.id, Arc::new(Vec::new())).await.unwrap();
    assert_eq!(done.state, JobState::Done);

    let snap = m.snapshot(&rec.id).unwrap();
    assert!(snap
        .chunks
        .iter()
        .all(|c| c.state == ium_core::chunk::ChunkState::Verified));
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        20 * MIB
    );
    assert!(snap.events.iter().any(|(_, k, _)| k == "verified"));
    assert!(snap.events.iter().any(|(_, k, _)| k == "state"));
}

#[tokio::test]
async fn zero_byte_job_completes_trivially() {
    let m = manager_with(Capability::new(Capability::RESUMABLE));
    let rec = m.create_job(&spec(0), 4).unwrap();
    let done = m.run_job(&rec.id, Arc::new(Vec::new())).await.unwrap();
    assert_eq!(done.state, JobState::Done);
    assert!(m.snapshot(&rec.id).unwrap().chunks.is_empty());
}

#[tokio::test]
async fn transient_failure_is_retried_and_job_still_completes() {
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(2), // two blips before success
        chunk_delay_ms: 0,
    }));
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 8,
        backoff_base_ms: 1, // keep the test fast
        backoff_cap_ms: 10,
        respect_retry_after: true,
    });

    let rec = m.create_job(&spec(6 * MIB), 3).unwrap();
    let done = m.run_job(&rec.id, Arc::new(Vec::new())).await.unwrap();
    assert_eq!(done.state, JobState::Done);

    let snap = m.snapshot(&rec.id).unwrap();
    assert!(snap
        .events
        .iter()
        .any(|(_, k, p)| k == "chunk_error" && p.contains("Transient")));
}

#[tokio::test]
async fn one_shot_endpoint_with_progress_becomes_unresumable() {
    // Adapter that probe-reports resumable=false AFTER some bytes confirmed:
    // simulate by manually journaling a confirm, then re-running.
    let m = manager_with(Capability::default()); // one-shot: no RESUMABLE flag
    let rec = m.create_job(&spec(10 * MIB), 2).unwrap();
    assert_eq!(rec.capability_badge, "one_shot");

    // pretend one chunk got fully confirmed before the session died
    m.journal()
        .update_chunk(
            &rec.id,
            0,
            ium_core::chunk::ChunkState::Uploaded,
            5 * MIB,
            None,
        )
        .unwrap();
    // force re-probe path: put the job back to Created
    m.journal()
        .set_job_state(&rec.id, JobState::Created)
        .unwrap();

    let out = m.run_job(&rec.id, Arc::new(Vec::new())).await.unwrap();
    assert_eq!(
        out.state,
        JobState::Unresumable,
        "one-shot + confirmed bytes = never restart from zero"
    );
}

#[tokio::test]
async fn pause_between_chunks_returns_paused_then_resume_completes() {
    use std::time::Duration;

    // 15 MiB with 5 MiB minimum → exactly 3 chunks; each upload takes ~20 ms so
    // the pause request (fired at ~5 ms) lands between chunk 1 and chunk 2.
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut jm = JobManager::with_journal(journal).unwrap();
    jm.register_adapter(Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(0),
        chunk_delay_ms: 20,
    }));
    let m = Arc::new(jm);
    let rec = m.create_job(&spec(15 * MIB), 3).unwrap();
    assert_eq!(m.snapshot(&rec.id).unwrap().chunks.len(), 3);

    let m2 = m.clone();
    let id = rec.id.clone();
    let pauser = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        m2.pause(&id).unwrap();
    });

    let out = m.run_job(&rec.id, Arc::new(Vec::new())).await.unwrap();
    pauser.await.unwrap();
    assert_eq!(out.state, JobState::Paused);

    // some work happened but not all
    let snap = m.snapshot(&rec.id).unwrap();
    let uploaded = snap
        .chunks
        .iter()
        .filter(|c| c.state != ium_core::chunk::ChunkState::Pending)
        .count();
    assert!(
        uploaded > 0,
        "pause happens between chunks, after some work"
    );
    assert!(
        uploaded < snap.chunks.len(),
        "pause must not run to completion"
    );

    // resume: clear flag and drive again
    m.resume_signal(&rec.id).unwrap();
    let done = m.run_job(&rec.id, Arc::new(Vec::new())).await.unwrap();
    assert_eq!(done.state, JobState::Done);
}

#[tokio::test]
async fn journal_replay_rebuilds_pool_and_only_finishes_unconfirmed() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("ium.db");

    // Session 1: create job, confirm one chunk by hand (partial progress), "crash".
    // 20 MiB / 4 conns with a 5 MiB minimum → exactly 4 chunks of 5 MiB.
    {
        let journal = SessionJournal::open(&db_path).unwrap();
        let mut m = JobManager::with_journal(journal).unwrap();
        m.register_adapter(Arc::new(MockAdapter {
            cap: Capability::new(Capability::RESUMABLE),
            fail_first: AtomicU32::new(0),
            chunk_delay_ms: 0,
        }));
        let rec = m.create_job(&spec(20 * MIB), 4).unwrap();
        assert_eq!(m.snapshot(&rec.id).unwrap().chunks.len(), 4);
        m.journal()
            .update_chunk(
                &rec.id,
                1,
                ium_core::chunk::ChunkState::Uploaded,
                MIB,
                Some("tok-1"),
            )
            .unwrap();
        m.journal().checkpoint_wal().unwrap();
    }

    // Session 2: reopen from the same file; run to done.
    let journal = SessionJournal::open(&db_path).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(0),
        chunk_delay_ms: 0,
    }));
    let jobs = m.list_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    let done = m
        .run_job(&jobs[0].id.clone(), Arc::new(Vec::new()))
        .await
        .unwrap();
    assert_eq!(done.state, JobState::Done);

    let snap = m.snapshot(&jobs[0].id).unwrap();
    let tok1 = snap.chunks.iter().find(|c| c.idx == 1).unwrap();
    assert_eq!(
        tok1.adapter_token.as_deref(),
        Some("tok-1"),
        "adapter token survived the crash"
    );
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        20 * MIB
    );
}

#[tokio::test]
async fn unknown_adapter_is_rejected_at_creation() {
    let journal = SessionJournal::open_in_memory().unwrap();
    let m = JobManager::with_journal(journal).unwrap();
    let mut s = spec(1024);
    s.adapter = "does_not_exist".into();
    assert!(matches!(
        m.create_job(&s, 2),
        Err(ium_core::EngineError::UnknownAdapter(_))
    ));
}

#[tokio::test]
async fn snapshot_of_missing_job_is_not_found() {
    let journal = SessionJournal::open_in_memory().unwrap();
    let m = JobManager::with_journal(journal).unwrap();
    assert!(matches!(
        m.snapshot("01AAAAAAAAAAAAAAAAAAAAAAAA"),
        Err(ium_core::EngineError::JobNotFound(_))
    ));
}
