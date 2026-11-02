//! Supervisor (issue #15): the spawn/pause/resume/cancel layer the desktop
//! shell drives. It lives in `ium-core` so every semantic is testable here,
//! without a webview — the Tauri glue stays thin (state extraction only).
//!
//! RED at current HEAD: `ium_core::supervisor` does not exist; nothing in
//! the engine (and nothing in the frozen shell) ever spawns `run_job`, so a
//! created job parks in `Created` forever. These tests define the contract
//! that fixes it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::supervisor::Supervisor;
use ium_core::{JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;

/// Minimal mock: real bytes off disk, wedge-at-entry for the cancel drill,
/// honest session semantics (fresh process = no session, like the real
/// adapters, issue #12).
struct WedgeMock {
    wedge_idx: std::sync::atomic::AtomicU32,
    wedge_release: Arc<tokio::sync::Notify>,
    wedge_engaged: Arc<AtomicBool>,
}

impl WedgeMock {
    fn new() -> Self {
        Self {
            wedge_idx: std::sync::atomic::AtomicU32::new(u32::MAX),
            wedge_release: Arc::new(tokio::sync::Notify::new()),
            wedge_engaged: Arc::new(AtomicBool::new(false)),
        }
    }
    fn arm_wedge(&self, idx: u32) -> (Arc<tokio::sync::Notify>, Arc<AtomicBool>) {
        self.wedge_idx.store(idx, Ordering::SeqCst);
        (self.wedge_release.clone(), self.wedge_engaged.clone())
    }
}

#[async_trait::async_trait]
impl TransferAdapter for WedgeMock {
    fn name(&self) -> &'static str {
        "wedge_mock"
    }
    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            max_parallel: None,
            session_token: Some("wedge-mock".into()),
        })
    }
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&spec.source_path)
            .map_err(|e| AdapterError::Transient(format!("source vanished: {e}")))?;
        file.seek(SeekFrom::Start(lease.resume_from)).unwrap();
        let want = (lease.end - lease.resume_from) as usize;
        let mut buf = vec![0u8; want];
        file.read_exact(&mut buf)
            .map_err(|e| AdapterError::Transient(format!("short read: {e}")))?;
        if self.wedge_idx.load(Ordering::SeqCst) == lease.idx {
            self.wedge_engaged.store(true, Ordering::SeqCst);
            self.wedge_release.notified().await;
        }
        progress.add(want as u64);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: lease.end - lease.resume_from,
            adapter_token: Some(format!("tok-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("wedge mock ok".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn make_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut bytes = Vec::with_capacity(size as usize);
    for off in 0..size {
        bytes.push((off % 251) as u8);
    }
    std::fs::write(&path, &bytes).unwrap();
    path
}

fn spec_for(path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "wedge_mock".into(),
        endpoint: "mock://endpoint".into(),
    }
}

fn supervisor_with(adapter: Arc<WedgeMock>) -> (Supervisor, Arc<JobManager>) {
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(adapter);
    let m = Arc::new(m);
    (Supervisor::new(Arc::clone(&m)), m)
}

#[tokio::test]
async fn supervisor_drive_spawns_the_transfer_and_reaches_done() {
    // The shell's contract: create a job, call drive(), walk away. Today
    // NOTHING spawns run_job (RED: this API does not exist) — a job would
    // sit in Created forever behind the frozen UI.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "driven.bin", 12 * MIB);
    let (sup, m) = supervisor_with(Arc::new(WedgeMock::new()));

    let rec = m.create_job(&spec_for(&path, 12 * MIB), 2).unwrap();
    sup.drive(&rec.id);

    // Journal-truth polling, exactly what the UI's job_updated watcher does.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let state = m.snapshot(&rec.id).unwrap().job.state;
        if state == JobState::Done || tokio::time::Instant::now() > deadline {
            assert_eq!(state, JobState::Done, "driven job must reach Done");
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn cancel_mid_running_lands_canceled_in_the_journal_and_survives_reopen() {
    // Cancel is a FIRST-CLASS terminal state: journaled BEFORE the drive
    // task is aborted (ADR-002), legal from Created/Probing/Running/Paused,
    // and it survives a fresh process — journal truth, not task truth.
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("cancel.db");
    let path = make_source(dir.path(), "cancel.bin", 12 * MIB);

    let mock = Arc::new(WedgeMock::new());
    let (_release, engaged) = mock.arm_wedge(0);
    let journal = SessionJournal::open(&journal_path).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    let adapter: Arc<dyn ium_core::TransferAdapter> = mock.clone();
    m.register_adapter(adapter);
    let m = Arc::new(m);
    let sup = Supervisor::new(Arc::clone(&m));

    let rec = m.create_job(&spec_for(&path, 12 * MIB), 3).unwrap();
    sup.drive(&rec.id);

    // Wait for the wedge: chunk 0 is mid-body (Running, worker parked).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !engaged.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "wedge never engaged — job never went Running"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let canceled = sup.cancel(&rec.id).await.unwrap();
    assert_eq!(canceled.state, JobState::Canceled);
    assert!(canceled.state.is_terminal());

    // Fresh process, same journal: the Canceled state IS the truth.
    drop(sup);
    let m2 = JobManager::open(&journal_path).unwrap();
    let reopened = m2.snapshot(&rec.id).unwrap().job;
    assert_eq!(reopened.state, JobState::Canceled);
    // A canceled job stays canceled: driving it is a no-op returning it.
    let again = m2.run_job(&rec.id).await.unwrap();
    assert_eq!(again.state, JobState::Canceled);
}

#[tokio::test]
async fn cancel_after_done_is_rejected_and_cancel_is_never_a_failure() {
    // Terminal states are past canceling; and a canceled job must never be
    // reported as Failed (the honest-badge rule applies to endings too).
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "done.bin", 8 * MIB);
    let (sup, m) = supervisor_with(Arc::new(WedgeMock::new()));

    let rec = m.create_job(&spec_for(&path, 8 * MIB), 2).unwrap();
    sup.drive(&rec.id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if m.snapshot(&rec.id).unwrap().job.state == JobState::Done
            || tokio::time::Instant::now() > deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(m.snapshot(&rec.id).unwrap().job.state, JobState::Done);

    let err = sup.cancel(&rec.id).await.unwrap_err();
    assert!(
        matches!(err, ium_core::EngineError::WrongState(_, _, _)),
        "canceling a Done job is a WrongState, got {err:?}"
    );
    // And the journal still says Done — never Failed-by-cancel.
    assert_eq!(m.snapshot(&rec.id).unwrap().job.state, JobState::Done);
}

#[tokio::test]
async fn pause_then_resume_through_the_supervisor_re_spawns_to_done() {
    // Resume is not a flag flip: the supervisor must RE-SPAWN the drive
    // task (the old one exited when it journaled Paused).
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "resumed.bin", 12 * MIB);
    let mock = Arc::new(WedgeMock::new());
    let (release, engaged) = mock.arm_wedge(0);
    let (sup, m) = supervisor_with(Arc::clone(&mock));

    let rec = m.create_job(&spec_for(&path, 12 * MIB), 1).unwrap();
    sup.drive(&rec.id);

    // Deterministic mid-run point: chunk 0 parked mid-body (Running).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !engaged.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "wedge never engaged"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sup.pause(&rec.id).unwrap();
    // Release the wedge; the worker finishes its chunk, the round ends and
    // the drive loop journals Paused instead of leasing anything new.
    release.notify_one();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if m.snapshot(&rec.id).unwrap().job.state == JobState::Paused
            || tokio::time::Instant::now() > deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(m.snapshot(&rec.id).unwrap().job.state, JobState::Paused);

    sup.resume_signal(&rec.id).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let state = m.snapshot(&rec.id).unwrap().job.state;
        if state == JobState::Done || tokio::time::Instant::now() > deadline {
            assert_eq!(state, JobState::Done, "resumed job must finish");
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
