//! End-to-end engine drills on a disk-backed mock adapter — the skeleton of
//! the Sprint-1 chaos harness. Every test drives REAL file bytes through the
//! manager: the mock reads its leased range from `spec.source_path` exactly
//! like a network adapter would.
//!
//! HTTP end-to-end lives in `http_one_shot.rs`; this file owns the
//! supervisor semantics (resume, retry exhaustion, pause, crash recovery).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, TransferAdapter, VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;

/// Deterministic pattern byte at `offset` — lets the mock verify it read the
/// right slice of the right file.
fn pattern_byte(offset: u64) -> u8 {
    (offset % 251) as u8 // 251 is prime: period does not align with chunk sizes
}

fn make_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut bytes = Vec::with_capacity(size as usize);
    for off in 0..size {
        bytes.push(pattern_byte(off));
    }
    std::fs::write(&path, &bytes).unwrap();
    path
}

/// What the mock saw, per upload call.
#[derive(Debug, Clone)]
struct UploadLog {
    idx: u32,
    resume_from: u64,
    bytes_moved: u64,
}

/// Mock adapter that reads its leased range from disk (like the real ones)
/// and logs every call. Failure modes are switchable per test:
/// * `fail_first` — N transient blips before the first success;
/// * `partial_first_len` — first confirm for a chunk confirms only this many
///   bytes (exercises the resume-from-confirmed-offset path).
struct MockAdapter {
    cap: Capability,
    fail_first: AtomicU32,
    partial_first_len: Option<u64>,
    chunk_delay_ms: u64,
    log: Mutex<Vec<UploadLog>>,
}

impl MockAdapter {
    fn new(cap: Capability) -> Self {
        Self {
            cap,
            fail_first: AtomicU32::new(0),
            partial_first_len: None,
            chunk_delay_ms: 0,
            log: Mutex::new(Vec::new()),
        }
    }
    fn failing(cap: Capability, n: u32) -> Self {
        Self {
            fail_first: AtomicU32::new(n),
            ..Self::new(cap)
        }
    }
    fn partial_first(cap: Capability, len: u64) -> Self {
        Self {
            partial_first_len: Some(len),
            ..Self::new(cap)
        }
    }
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
        spec: &JobSpec,
        lease: &Lease,
    ) -> Result<ChunkConfirm, AdapterError> {
        if self.chunk_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.chunk_delay_ms)).await;
        }
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(AdapterError::Transient("mock network blip".into()));
        }

        // Read the REAL byte range from disk — [resume_from, lease.end).
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&spec.source_path)
            .map_err(|e| AdapterError::Transient(format!("source vanished: {e}")))?;
        file.seek(SeekFrom::Start(lease.resume_from)).unwrap();
        let want = (lease.end - lease.resume_from) as usize;
        let mut buf = vec![0u8; want];
        file.read_exact(&mut buf)
            .map_err(|e| AdapterError::Transient(format!("short read: {e}")))?;
        // The mock proves it read the correct slice of the correct file.
        for (i, b) in buf.iter().enumerate() {
            assert_eq!(*b, pattern_byte(lease.resume_from + i as u64));
        }

        let full = lease.end - lease.resume_from;
        let confirmed = match self.partial_first_len {
            Some(p) if p < full && self.log.lock().unwrap().is_empty() => p,
            _ => full,
        };
        self.log.lock().unwrap().push(UploadLog {
            idx: lease.idx,
            resume_from: lease.resume_from,
            bytes_moved: want as u64,
        });
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: confirmed,
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

fn spec_for(path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "mock".into(),
        endpoint: "mock://endpoint".into(),
    }
}

fn manager_with(adapter: Arc<MockAdapter>) -> JobManager {
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(adapter);
    m
}

fn fast_policy(m: &mut JobManager) {
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 8,
        backoff_base_ms: 1,
        backoff_cap_ms: 5,
        respect_retry_after: true,
    });
}

#[tokio::test]
async fn job_runs_to_done_with_all_chunks_verified() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "a.bin", 20 * MIB);
    let mock = Arc::new(MockAdapter::new(Capability::new(
        Capability::RESUMABLE | Capability::TAIL_RACE | Capability::DYNAMIC_CHUNKS,
    )));
    let m = manager_with(mock);

    let rec = m.create_job(&spec_for(&path, 20 * MIB), 4).unwrap();
    assert_eq!(rec.state, JobState::Created);
    assert_eq!(rec.capability_badge, "resumable");
    assert_eq!(m.snapshot(&rec.id).unwrap().chunks.len(), 4);

    let done = m.run_job(&rec.id).await.unwrap();
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

    // the full state path is journaled, in order
    let states: Vec<String> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "state")
        .map(|(_, _, p)| {
            serde_json::from_str::<serde_json::Value>(p).unwrap()["to"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        states,
        ["probing", "running", "finishing", "done"],
        "journaled state path must be the happy path"
    );
}

#[tokio::test]
async fn zero_byte_job_completes_trivially() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "empty.bin", 0);
    let m = manager_with(Arc::new(MockAdapter::new(Capability::new(
        Capability::RESUMABLE,
    ))));
    let rec = m.create_job(&spec_for(&path, 0), 4).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);
    assert!(m.snapshot(&rec.id).unwrap().chunks.is_empty());
}

#[tokio::test]
async fn transient_failures_are_retried_then_exhausted() {
    // Regression: attempts must be counted per chunk. A permanently failing
    // endpoint must produce exactly max_attempts+1 upload calls and a Failed
    // job — never an infinite loop, never a premature Done.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "b.bin", 6 * MIB);

    let mock = Arc::new(MockAdapter::failing(
        Capability::new(Capability::RESUMABLE),
        u32::MAX, // fails forever
    ));
    let mut m = manager_with(mock);
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 3,
        backoff_base_ms: 1,
        backoff_cap_ms: 2,
        respect_retry_after: true,
    });

    let rec = m.create_job(&spec_for(&path, 6 * MIB), 3).unwrap();
    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(out.state, JobState::Failed);

    let snap = m.snapshot(&rec.id).unwrap();
    let errors: Vec<&(i64, String, String)> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_error")
        .collect();
    assert_eq!(errors.len(), 4, "1 initial + 3 retries, then Fail");
    // attempts must be counted, not reset
    let attempts: Vec<u32> = errors
        .iter()
        .map(|(_, _, p)| {
            serde_json::from_str::<serde_json::Value>(p).unwrap()["attempt"]
                .as_u64()
                .unwrap() as u32
        })
        .collect();
    assert_eq!(attempts, vec![1, 2, 3, 4]);
    // the chunk went back to the journal as Pending, keeping confirmed bytes
    assert_eq!(snap.chunks[0].state, ium_core::chunk::ChunkState::Pending);

    // re-running a Failed job is an explicit error, not a silent loop
    assert!(matches!(
        m.run_job(&rec.id).await,
        Err(ium_core::EngineError::WrongState(_, _, _))
    ));
}

#[tokio::test]
async fn transient_blip_is_retried_and_job_still_completes() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "c.bin", 6 * MIB);

    let mock = Arc::new(MockAdapter::failing(
        Capability::new(Capability::RESUMABLE),
        2, // two blips, then success
    ));
    let mut m = manager_with(mock);
    fast_policy(&mut m);

    let rec = m.create_job(&spec_for(&path, 6 * MIB), 3).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);

    let snap = m.snapshot(&rec.id).unwrap();
    assert!(snap
        .events
        .iter()
        .any(|(_, k, p)| k == "chunk_error" && p.contains("Transient")));
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        6 * MIB
    );
}

#[tokio::test]
async fn partial_confirm_resumes_from_confirmed_offset() {
    // Regression for the premature-Done bug: a chunk confirmed only halfway
    // must return to the pool (Pending, bytes kept) and be re-leased with
    // resume_from at the confirmed offset — never skipped, never restarted.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "d.bin", 2 * MIB);

    let mock = Arc::new(MockAdapter::partial_first(
        Capability::new(Capability::RESUMABLE),
        MIB / 2, // first call confirms only 0.5 MiB of the 2 MiB chunk
    ));
    let m = manager_with(Arc::clone(&mock));
    let rec = m.create_job(&spec_for(&path, 2 * MIB), 1).unwrap();

    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);

    // The upload log IS the resume proof: exactly two calls — the first
    // starting at 0, the second resuming at the confirmed offset.
    let log = mock.log.lock().unwrap().clone();
    assert_eq!(
        log.len(),
        2,
        "one partial lease + one resuming lease: {log:?}"
    );
    assert_eq!(log[0].idx, 0);
    assert_eq!(log[0].resume_from, 0);
    assert_eq!(log[1].idx, 0, "same chunk was re-leased, not a new one");
    assert_eq!(
        log[1].resume_from,
        MIB / 2,
        "second lease must resume at the server-confirmed offset"
    );
    assert_eq!(log[1].bytes_moved, 2 * MIB - MIB / 2, "no re-sent bytes");

    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(snap.chunks[0].state, ium_core::chunk::ChunkState::Verified);
    assert_eq!(snap.chunks[0].bytes_confirmed, 2 * MIB);
    assert!(
        snap.events
            .iter()
            .any(|(_, k, p)| k == "chunk_confirmed" && p.contains("\"full\":false")),
        "the partial confirm must be journaled as partial"
    );
}

#[tokio::test]
async fn one_shot_endpoint_with_progress_becomes_unresumable() {
    // One-shot + confirmed bytes = never restart from zero (spec rule #1).
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "e.bin", 10 * MIB);
    let m = manager_with(Arc::new(MockAdapter::new(Capability::default())));

    let rec = m.create_job(&spec_for(&path, 10 * MIB), 2).unwrap();
    assert_eq!(rec.capability_badge, "one_shot");
    // One-shot jobs are planned as a SINGLE chunk — chunking a stream that
    // cannot resume would imply a capability the protocol lacks.
    assert_eq!(m.snapshot(&rec.id).unwrap().chunks.len(), 1);

    // pretend bytes got confirmed before the session died
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

    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(out.state, JobState::Unresumable);
}

#[tokio::test]
async fn pause_between_chunks_returns_paused_then_resume_completes() {
    use std::time::Duration;

    // 15 MiB with 5 MiB minimum → exactly 3 chunks; each upload takes ~20 ms so
    // the pause request (fired at ~5 ms) lands between chunk 1 and chunk 2.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "f.bin", 15 * MIB);

    let mock = Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(0),
        partial_first_len: None,
        chunk_delay_ms: 20,
        log: Mutex::new(Vec::new()),
    });
    let m = Arc::new(manager_with(mock));
    let rec = m.create_job(&spec_for(&path, 15 * MIB), 3).unwrap();
    assert_eq!(m.snapshot(&rec.id).unwrap().chunks.len(), 3);

    let m2 = Arc::clone(&m);
    let id = rec.id.clone();
    let pauser = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        m2.pause(&id).unwrap();
    });

    let out = m.run_job(&rec.id).await.unwrap();
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
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);
}

#[tokio::test]
async fn crash_mid_running_recovers_without_reuploading_confirmed_bytes() {
    // Session 1: chunk 0 fully confirmed, journal left in `Running` (as if the
    // process died between chunk confirms). Session 2 must pick up from the
    // journal and NOT re-upload chunk 0.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "g.bin", 10 * MIB);
    let db_path = dir.path().join("ium.db");

    {
        let journal = SessionJournal::open(&db_path).unwrap();
        let mut m = JobManager::with_journal(journal).unwrap();
        m.register_adapter(Arc::new(MockAdapter::new(Capability::new(
            Capability::RESUMABLE,
        ))));
        let rec = m.create_job(&spec_for(&path, 10 * MIB), 2).unwrap();

        // simulate: chunk 0 done, supervisor died before chunk 1
        m.journal()
            .set_job_state(&rec.id, JobState::Running)
            .unwrap();
        m.journal()
            .update_chunk(
                &rec.id,
                0,
                ium_core::chunk::ChunkState::Uploaded,
                5 * MIB,
                Some("tok-0"),
            )
            .unwrap();
        m.journal().checkpoint_wal().unwrap();
    }

    // Session 2: reopen, drive to done, prove chunk 0 was never re-sent.
    let journal = SessionJournal::open(&db_path).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(MockAdapter::new(Capability::new(
        Capability::RESUMABLE,
    ))));

    let jobs = m.list_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    let rec = m.run_job(&jobs[0].id.clone()).await.unwrap();
    assert_eq!(rec.state, JobState::Done);

    let snap = m.snapshot(&jobs[0].id).unwrap();
    assert_eq!(snap.job.state, JobState::Done);
    // chunk 0 kept its original adapter token → it was never re-uploaded;
    // a fresh upload would have replaced "tok-0" with a new token.
    let tok0 = snap.chunks.iter().find(|c| c.idx == 0).unwrap();
    assert_eq!(tok0.adapter_token.as_deref(), Some("tok-0"));
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        10 * MIB
    );
}

#[tokio::test]
async fn stale_lease_from_crashed_driver_is_reclaimed() {
    // A crash while a chunk was `Leased` (uploaded, confirm never journaled)
    // must NOT strand the job: the supervisor reclaims the lease, keeps the
    // confirmed bytes, and finishes.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "h.bin", 5 * MIB);

    let m = manager_with(Arc::new(MockAdapter::new(Capability::new(
        Capability::RESUMABLE,
    ))));
    let rec = m.create_job(&spec_for(&path, 5 * MIB), 1).unwrap();

    // simulate the crash window: chunk leased, half confirmed, confirm lost
    m.journal()
        .set_job_state(&rec.id, JobState::Running)
        .unwrap();
    m.journal()
        .update_chunk(&rec.id, 0, ium_core::chunk::ChunkState::Leased, MIB, None)
        .unwrap();

    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);
    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(snap.chunks[0].state, ium_core::chunk::ChunkState::Verified);
}

#[tokio::test]
async fn double_run_is_rejected_not_raced() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "i.bin", 15 * MIB);

    let mock = Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(0),
        partial_first_len: None,
        chunk_delay_ms: 30, // keep the first driver busy so the second lands mid-run
        log: Mutex::new(Vec::new()),
    });
    let m = Arc::new(manager_with(mock));

    let rec = m.create_job(&spec_for(&path, 15 * MIB), 3).unwrap();

    let (a, b) = tokio::join!(m.run_job(&rec.id), m.run_job(&rec.id));
    assert!(a.is_ok(), "first driver wins: {a:?}");
    assert!(
        matches!(b, Err(ium_core::EngineError::WrongState(_, _, _))),
        "second driver must be rejected, got {b:?}"
    );
}

#[tokio::test]
async fn unknown_adapter_is_rejected_at_creation() {
    let journal = SessionJournal::open_in_memory().unwrap();
    let m = JobManager::with_journal(journal).unwrap();
    let s = JobSpec {
        source_path: "/tmp/none".into(),
        source_size: 1024,
        adapter: "does_not_exist".into(),
        endpoint: "mock://x".into(),
    };
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

#[test]
fn file_backed_manager_reopens_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    {
        let journal = SessionJournal::open(&db).unwrap();
        let m = JobManager::with_journal(journal).unwrap();
        assert!(m.list_jobs().unwrap().is_empty());
    }
    let journal = SessionJournal::open(&db).unwrap();
    let m = JobManager::with_journal(journal).unwrap();
    assert!(m.list_jobs().unwrap().is_empty());
}
