//! End-to-end engine drills on a disk-backed mock adapter — the skeleton of
//! the Sprint-1 chaos harness. Every test drives REAL file bytes through the
//! manager: the mock reads its leased range from `spec.source_path` exactly
//! like a network adapter would.
//!
//! HTTP end-to-end lives in `http_one_shot.rs`; this file owns the
//! supervisor semantics (resume, retry exhaustion, pause, crash recovery).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
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

/// RAII seat in `upload_chunk`: bumps `in_flight` on construction, records
/// the high-water mark, releases the seat on drop (every exit path — blip,
/// session loss, wedge — must give the seat back).
struct InFlightGuard<'a> {
    count: &'a AtomicUsize,
}

impl<'a> InFlightGuard<'a> {
    fn new(count: &'a AtomicUsize, max: &'a AtomicUsize) -> Self {
        let now = count.fetch_add(1, Ordering::SeqCst) + 1;
        max.fetch_max(now, Ordering::SeqCst);
        Self { count }
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Mock adapter that reads its leased range from disk (like the real ones)
/// and logs every call AT ENTRY (parity with ScriptedAdapter): a call that
/// blips, hits a vanished source or never completes stays visible with
/// `bytes_moved: 0`; the real figure is patched in at completion.
/// Failure modes are switchable per test:
/// * `fail_first` — N transient blips before the first success;
/// * `partial_first_len` — first confirm for a chunk confirms only this many
///   bytes (exercises the resume-from-confirmed-offset path).
/// * `fail_session_at_call` — upload call N returns `SessionLost` (the
///   server lost the session mid-transfer); every other call succeeds.
/// * `fail_session_forever` — EVERY upload call returns `SessionLost`
///   (the session is really gone; recovery must exhaust, not spin).
///
/// Session semantics model the real S3/TUS adapters (issue #12):
/// `has_session` starts **false** — a fresh process's adapter session map is
/// EMPTY; `probe` sets it (probe = CreateMultipartUpload / POST creation);
/// and `verify` errors `SessionLost` when it is false — exactly what
/// `adapter_s3`/`adapter_tus` do when their session map misses.
struct MockAdapter {
    cap: Capability,
    fail_first: AtomicU32,
    partial_first_len: Option<u64>,
    chunk_delay_ms: u64,
    /// Per-call ceiling the probe declares (issue #8: adapter declares,
    /// planner obeys). `None` = undeclared, today's behavior.
    probe_cap: Option<u64>,
    /// Concurrency ceiling the probe declares (issue #13). `None` =
    /// undeclared.
    probe_max_parallel: Option<u32>,
    log: Mutex<Vec<UploadLog>>,
    has_session: std::sync::atomic::AtomicBool,
    fail_session_at_call: AtomicU32,
    fail_session_forever: std::sync::atomic::AtomicBool,
    reattach_count: AtomicU32,
    /// Upload calls currently inside `upload_chunk` (issue #13 RED probe).
    in_flight: AtomicUsize,
    /// High-water mark of `in_flight` — the observed fleet concurrency.
    max_in_flight: AtomicUsize,
    /// When `lease.idx == wedge_idx` the call parks at entry until released
    /// through `wedge_release` (deterministic mid-body state, drill style).
    wedge_idx: AtomicU32,
    wedge_release: Arc<tokio::sync::Notify>,
    wedge_engaged: Arc<AtomicBool>,
}

impl MockAdapter {
    fn new(cap: Capability) -> Self {
        Self {
            cap,
            fail_first: AtomicU32::new(0),
            partial_first_len: None,
            chunk_delay_ms: 0,
            probe_cap: None,
            probe_max_parallel: None,
            log: Mutex::new(Vec::new()),
            has_session: std::sync::atomic::AtomicBool::new(false),
            fail_session_at_call: AtomicU32::new(0),
            fail_session_forever: std::sync::atomic::AtomicBool::new(false),
            reattach_count: AtomicU32::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            wedge_idx: AtomicU32::new(u32::MAX),
            wedge_release: Arc::new(tokio::sync::Notify::new()),
            wedge_engaged: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Declare `max_chunk_bytes` from the probe (issue #8).
    fn with_probe_cap(mut self, cap: Option<u64>) -> Self {
        self.probe_cap = cap;
        self
    }
    /// Declare `max_parallel` from the probe (issue #13).
    fn with_probe_max_parallel(mut self, n: Option<u32>) -> Self {
        self.probe_max_parallel = n;
        self
    }
    /// Park the call for `idx` at entry until `release()` is notified.
    /// Returns the release handle and the engaged flag (observable from the
    /// test without polling the log).
    fn arm_wedge(&self, idx: u32) -> (Arc<tokio::sync::Notify>, Arc<AtomicBool>) {
        self.wedge_idx.store(idx, Ordering::SeqCst);
        (self.wedge_release.clone(), self.wedge_engaged.clone())
    }
    fn release_wedge(&self) {
        self.wedge_release.notify_one();
    }
    /// Observed peak concurrency of `upload_chunk` calls.
    fn max_concurrent_seen(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }
    /// Upload call `call` returns `SessionLost` (issue #12).
    fn with_session_loss_at(self, call: u32) -> Self {
        self.fail_session_at_call.store(call, Ordering::SeqCst);
        self
    }
    /// Every upload call returns `SessionLost` (issue #12).
    fn with_session_lost_forever(self) -> Self {
        self.fail_session_forever
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }
    fn reattach_count(&self) -> u32 {
        self.reattach_count.load(Ordering::SeqCst)
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
        // Probe honesty mirrors the declared capability (spec §5). Probe
        // CREATES the session on resumable endpoints (S3 CreateMultipartUpload,
        // TUS POST creation) — the fresh map is no longer empty.
        self.has_session
            .store(self.cap.has(Capability::RESUMABLE), Ordering::SeqCst);
        Ok(ProbeReport {
            resumable: self.cap.has(Capability::RESUMABLE),
            max_chunk_bytes: self.probe_cap,
            min_chunk_bytes: None,
            max_parallel: self.probe_max_parallel,
            session_token: Some("mock-session".into()),
        })
    }
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        // Log at ENTRY: a call that never completes (blip, vanished source,
        // evicted mid-body) must still be visible to tests. `bytes_moved` is
        // patched with the real figure at completion, below.
        let entry = {
            let mut log = self.log.lock().unwrap();
            log.push(UploadLog {
                idx: lease.idx,
                resume_from: lease.resume_from,
                bytes_moved: 0,
            });
            log.len() - 1
        };
        // Issue #13 instrumentation: take a concurrency seat (tracks the
        // observed fleet peak), then honor the entry wedge if armed for
        // this chunk — a deterministic mid-body hold, drill style.
        let _in_flight = InFlightGuard::new(&self.in_flight, &self.max_in_flight);
        if self.wedge_idx.load(Ordering::SeqCst) == lease.idx {
            self.wedge_engaged.store(true, Ordering::SeqCst);
            self.wedge_release.notified().await;
        }
        if self.chunk_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.chunk_delay_ms)).await;
        }
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(AdapterError::Transient("mock network blip".into()));
        }
        // Issue #12 failure modes: the server lost the session.
        let call_no = (entry + 1) as u32; // entry log order == arrival order
        if self
            .fail_session_forever
            .load(std::sync::atomic::Ordering::SeqCst)
            || self.fail_session_at_call.load(Ordering::SeqCst) == call_no
        {
            return Err(AdapterError::SessionLost(format!(
                "mock: server lost the session (call {call_no})"
            )));
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
        // Wire telemetry: every byte read went "onto the wire".
        progress.add(want as u64);

        // Call completed: patch the entry created above with the real figure.
        self.log.lock().unwrap()[entry].bytes_moved = want as u64;

        let full = lease.end - lease.resume_from;
        let confirmed = match self.partial_first_len {
            // "first call" = the only log entry is the one this call pushed
            // at entry (len() == 1, not is_empty() — the log is now written
            // at entry, see above).
            Some(p) if p < full && self.log.lock().unwrap().len() == 1 => p,
            _ => full,
        };
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: confirmed,
            adapter_token: Some(format!("tok-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        // Model the real adapters: verify without a session (fresh process,
        // map empty) is SessionLost — never a silent pass (issue #12).
        if !self.has_session.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AdapterError::SessionLost(
                "mock: no session in the fresh process's adapter map".into(),
            ));
        }
        Ok(VerifyReport {
            ok: true,
            detail: Some("mock hash ok".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        // Model restore_session: the journaled token resurrects the session
        // in this (possibly fresh) adapter's map (issue #12).
        self.has_session
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.reattach_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn session_lost_mid_transfer_reattaches_instead_of_failing() {
    // Issue #12, hole 1 — the scratch test from the issue, committed for
    // real. The taxonomy promises `Action::Reattach` for SessionLost; the
    // manager used to treat it exactly like Fail, killing an otherwise
    // healthy resumable upload whose state is fully recoverable from the
    // journaled probe event. Mock: probe ok, upload call 2 SessionLost
    // (server lost the session), everything after succeeds; 8 attempts.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "sesslost.bin", 10 * MIB);
    let mock =
        Arc::new(MockAdapter::new(Capability::new(Capability::RESUMABLE)).with_session_loss_at(2));
    let mut m = manager_with(Arc::clone(&mock));
    fast_policy(&mut m);

    let rec = m.create_job(&spec_for(&path, 10 * MIB), 2).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(
        done.state,
        JobState::Done,
        "a recoverable SessionLost must not fail the job"
    );

    // The recovery is journaled and the token really replayed.
    let snap = m.snapshot(&rec.id).unwrap();
    assert!(
        snap.events.iter().any(|(_, k, _)| k == "reattach"),
        "the reattach must be journaled like every write"
    );
    assert!(
        mock.reattach_count() >= 1,
        "manager must have replayed the journaled token into the adapter"
    );

    // Wire proof: exactly 3 calls — ok, lost, recovered. The lost call
    // carries no confirmation, so its chunk re-leased from its BEGIN and
    // re-sent its full span (a SessionLost confirms zero bytes).
    let log = mock.log.lock().unwrap().clone();
    assert_eq!(log.len(), 3, "ok + lost + recovered: {log:?}");
    assert_eq!(log[1].bytes_moved, 0, "the SessionLost call moved nothing");
    let lost_idx = log[1].idx;
    assert_ne!(log[0].idx, lost_idx, "the sibling chunk completed: {log:?}");
    assert_eq!(log[2].idx, lost_idx, "the lost chunk was re-leased");
    assert_eq!(
        log[2].resume_from,
        lost_idx as u64 * 5 * MIB,
        "re-lease restarts at the chunk begin — zero server-confirmed bytes to keep"
    );
    assert_eq!(
        log[2].bytes_moved,
        5 * MIB,
        "full span re-sent, nothing more"
    );
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        10 * MIB
    );
}

#[tokio::test]
async fn finishing_entry_session_replay_verifies_in_fresh_process() {
    // Issue #12, hole 2: a crash between the last confirm and verify leaves
    // the job at `Finishing` with a journaled probe event; a fresh process's
    // adapter has an EMPTY session map, so verify used to SessionLost one
    // step before Done. The Finishing-entry replay must reattach from the
    // journal, then verify.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "finishing.bin", 10 * MIB);
    let db = dir.path().join("ium.db");

    // Session 1: really probe + upload everything (the probe event hits the
    // journal), then rewind to the crash window: all chunks Uploaded with
    // full bytes, job Finishing — exactly what a kill between the last
    // confirm and verify leaves behind.
    {
        let journal = SessionJournal::open(&db).unwrap();
        let mut m = JobManager::with_journal(journal).unwrap();
        m.register_adapter(Arc::new(MockAdapter::new(Capability::new(
            Capability::RESUMABLE,
        ))));
        let rec = m.create_job(&spec_for(&path, 10 * MIB), 2).unwrap();
        m.run_job(&rec.id).await.unwrap();
        m.journal()
            .set_job_state(&rec.id, JobState::Finishing)
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

    // Session 2: FRESH manager + FRESH adapter — session map empty, exactly
    // like a restarted process.
    let mock2 = Arc::new(MockAdapter::new(Capability::new(Capability::RESUMABLE)));
    let journal = SessionJournal::open(&db).unwrap();
    let mut m2 = JobManager::with_journal(journal).unwrap();
    m2.register_adapter(Arc::clone(&mock2) as Arc<dyn TransferAdapter>);

    let jobs = m2.list_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, JobState::Finishing);

    let out = m2.run_job(&jobs[0].id.clone()).await.unwrap();
    assert_eq!(
        out.state,
        JobState::Done,
        "verify must run on a replayed session, not die one step before Done"
    );
    assert_eq!(
        mock2.reattach_count(),
        1,
        "exactly one Finishing-entry replay from the journaled probe token"
    );
    let snap = m2.snapshot(&jobs[0].id).unwrap();
    assert!(
        snap.events
            .iter()
            .any(|(_, k, p)| k == "reattach" && p.contains("finishing_entry")),
        "the Finishing-entry replay must be journaled"
    );
    // Zero re-upload: everything was already confirmed in the journal.
    assert!(
        mock2.log.lock().unwrap().is_empty(),
        "a fresh Finishing-entry process must not upload a single byte"
    );
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        10 * MIB
    );
}

#[tokio::test]
async fn session_lost_forever_terminates_failed_not_spins() {
    // Issue #12 acceptance: `SessionLost`-for-real (token invalid even after
    // reattach) still terminates in Failed — recovery must not spin. The
    // reattach action consumes the SAME per-chunk retry budget as retries,
    // so the loop is bounded by max_attempts exactly like blips.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "sessgone.bin", 10 * MIB);
    let mock = Arc::new(
        MockAdapter::new(Capability::new(Capability::RESUMABLE)).with_session_lost_forever(),
    );
    let mut m = manager_with(Arc::clone(&mock));
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 2,
        backoff_base_ms: 1,
        backoff_cap_ms: 2,
        respect_retry_after: true,
    });

    let rec = m.create_job(&spec_for(&path, 10 * MIB), 1).unwrap();
    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(
        out.state,
        JobState::Failed,
        "recovery exhausted into Failed"
    );

    // Bounded: max_attempts reattach cycles for the chunk, then Fail.
    let snap = m.snapshot(&rec.id).unwrap();
    let reattaches = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "reattach")
        .count();
    assert_eq!(
        reattaches, 2,
        "reattach cycles are counted against the budget, not free"
    );
    let log = mock.log.lock().unwrap().clone();
    assert_eq!(log.len(), 3, "attempt 1 + 2 reattach cycles = 3 calls");
    assert!(log.iter().all(|l| l.bytes_moved == 0));
}

#[tokio::test]
async fn probe_max_chunk_bytes_caps_the_plan() {
    // Issue #8: the adapter declares a per-call ceiling at probe time; the
    // planner must obey (ADR-011). A 10 MiB source with max_conns = 2 plans
    // two 5 MiB chunks; a 4 MiB adapter cap must force >= 3 chunks, none
    // larger than the declared cap, and the re-plan must be journaled.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "capped.bin", 10 * MIB);
    let mock = Arc::new(
        MockAdapter::new(Capability::new(Capability::RESUMABLE)).with_probe_cap(Some(4 * MIB)),
    );
    let m = manager_with(mock);

    let rec = m.create_job(&spec_for(&path, 10 * MIB), 2).unwrap();
    assert_eq!(
        m.snapshot(&rec.id).unwrap().chunks.len(),
        2,
        "pre-probe plan is unconstrained (cap is not known yet)"
    );

    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);

    let snap = m.snapshot(&rec.id).unwrap();
    assert!(
        snap.chunks.len() >= 3,
        "cap 4 MiB on 10 MiB must raise n to >= 3, got {}",
        snap.chunks.len()
    );
    assert!(
        snap.chunks.iter().all(|c| c.end - c.begin <= 4 * MIB),
        "no chunk may exceed the adapter-declared cap"
    );
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        10 * MIB
    );
    assert!(
        snap.events.iter().any(|(_, k, _)| k == "replan_cap"),
        "the cap re-plan must be journaled like every write"
    );
}

#[tokio::test]
async fn probe_declared_max_parallel_one_forbids_a_concurrent_second_lease() {
    // Issue #13: TUS is ONE sequential-offset resource. An adapter that
    // declares max_parallel = Some(1) at probe time must clamp the drive
    // round to a single worker. Deterministic RED shape: chunk 0's call
    // parks at entry; while it is parked NO other upload call may exist.
    // Today's fleet spawns min(pending, MAX_WORKERS) workers regardless of
    // the declaration, so worker 2 leases chunk 1 mid-wedge and the
    // observed concurrency high-water mark reaches 2 — the assertion below
    // fails until the manager consumes the declaration.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "sequential.bin", 16 * MIB);

    let mock = Arc::new(
        MockAdapter::new(Capability::new(Capability::RESUMABLE)).with_probe_max_parallel(Some(1)),
    );
    let (_release, _engaged) = mock.arm_wedge(0);
    let m = manager_with(Arc::clone(&mock));

    // 16 MiB with max_conns = 4 plans multiple chunks; only the DRIVE round
    // may be clamped — the plan itself stays untouched (lease order is
    // lowest-idx-first, so one worker walks a TUS session in offset order).
    let rec = m.create_job(&spec_for(&path, 16 * MIB), 4).unwrap();
    assert!(
        m.snapshot(&rec.id).unwrap().chunks.len() > 1,
        "pre-probe plan is unconstrained — the clamp is a drive-round property"
    );

    // Release the wedge as soon as it engages, after a settle window that
    // gives a buggy concurrent lease ample time to show up (the buggy fleet
    // leases chunk 1 within microseconds of the round starting).
    let mock2 = Arc::clone(&mock);
    tokio::spawn(async move {
        for _ in 0..500 {
            if mock2.wedge_engaged.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        mock2.release_wedge();
    });

    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);
    assert_eq!(
        mock.max_concurrent_seen(),
        1,
        "a probe that declares max_parallel=1 must never see two upload calls in flight"
    );
    let log = mock.log.lock().unwrap().clone();
    assert_eq!(log.len(), m.snapshot(&rec.id).unwrap().chunks.len());
    assert_eq!(
        log.iter().map(|l| l.bytes_moved).sum::<u64>(),
        16 * MIB,
        "all bytes still moved — exactly once, sequentially"
    );
}

fn spec_for(path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "mock".into(),
        endpoint: "mock://endpoint".into(),
    }
}

fn scripted_spec_for(path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "mock_scripted".into(),
        endpoint: "mock://scripted".into(),
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
async fn zero_byte_job_uploads_and_verifies_like_any_file() {
    // Issue #5 invariant: even a 0-byte file plans one [0, 0) chunk that
    // must be leased, uploaded, confirmed and verified — the old empty-plan
    // convention let such a job reach Done with zero server round-trips.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "empty.bin", 0);
    let m = manager_with(Arc::new(MockAdapter::new(Capability::new(
        Capability::RESUMABLE,
    ))));
    let rec = m.create_job(&spec_for(&path, 0), 4).unwrap();
    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(snap.chunks.len(), 1);
    assert_eq!((snap.chunks[0].begin, snap.chunks[0].end), (0, 0));

    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);
    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(snap.chunks[0].state, ium_core::chunk::ChunkState::Verified);
    assert_eq!(snap.chunks[0].bytes_confirmed, 0);
    assert_eq!(
        snap.events
            .iter()
            .filter(|(_, k, _)| k == "verified")
            .count(),
        1,
        "verify-before-declare must have run"
    );
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
async fn mock_adapter_logs_calls_at_entry() {
    // Observability parity with ScriptedAdapter: a call that FAILS must
    // still appear in the mock's log — pushed at entry, `bytes_moved == 0`
    // until patched at completion. The old log-at-completion behavior made
    // blipped calls invisible (same gap that hid stalled/evicted calls
    // before the ScriptedAdapter fix).
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "g.bin", 6 * MIB);

    let mock = Arc::new(MockAdapter::failing(
        Capability::new(Capability::RESUMABLE),
        u32::MAX, // every call blips; none ever completes
    ));
    let mut m = manager_with(Arc::clone(&mock));
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 3,
        backoff_base_ms: 1,
        backoff_cap_ms: 2,
        respect_retry_after: true,
    });

    let rec = m.create_job(&spec_for(&path, 6 * MIB), 1).unwrap();
    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(out.state, JobState::Failed);

    // 1 initial + 3 retries = 4 upload calls — ALL visible from entry.
    let log = mock.log.lock().unwrap().clone();
    assert_eq!(log.len(), 4, "failed calls must be logged too: {log:?}");
    assert!(
        log.iter().all(|l| l.bytes_moved == 0),
        "a blipped call moved nothing: {log:?}"
    );
    assert!(log.iter().all(|l| l.idx == 0), "one chunk, four calls");
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

    // 15 MiB with 5 MiB minimum → exactly 3 chunks; each upload takes ~20 ms
    // and all three run CONCURRENTLY (one worker per chunk). The pause fires
    // at ~5 ms, mid-flight. Concurrent semantics: the in-flight chunks
    // finish (a cancelled one-shot body cannot be re-sent), the workers then
    // observe the flag and exit WITHOUT leasing anything new — so the job
    // ends `Paused`, never `Done`. That Paused-not-Done outcome IS the proof
    // the pause landed.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "f.bin", 15 * MIB);

    let mock = Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(0),
        partial_first_len: None,
        chunk_delay_ms: 20,
        probe_cap: None,
        probe_max_parallel: None,
        log: Mutex::new(Vec::new()),
        has_session: std::sync::atomic::AtomicBool::new(false),
        fail_session_at_call: AtomicU32::new(0),
        fail_session_forever: std::sync::atomic::AtomicBool::new(false),
        reattach_count: AtomicU32::new(0),
        in_flight: AtomicUsize::new(0),
        max_in_flight: AtomicUsize::new(0),
        wedge_idx: AtomicU32::new(u32::MAX),
        wedge_release: Arc::new(tokio::sync::Notify::new()),
        wedge_engaged: Arc::new(AtomicBool::new(false)),
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
    assert_eq!(out.state, JobState::Paused, "pause must stop the drive");

    // the pause request is journaled and in-flight work was completed
    let snap = m.snapshot(&rec.id).unwrap();
    assert!(snap.events.iter().any(|(_, k, _)| k == "pause_requested"));
    let uploaded = snap
        .chunks
        .iter()
        .filter(|c| c.state != ium_core::chunk::ChunkState::Pending)
        .count();
    assert!(uploaded > 0, "in-flight chunks completed before the pause");
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
    assert!(!states.contains(&"done".to_string()), "never reached Done");

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

        // simulate: chunk 0 done, supervisor died before chunk 1. The crash
        // window ALWAYS contains a probe event (run_job journals it before
        // Running) — the session token is what recovery replays (#12).
        m.journal()
            .set_job_state(&rec.id, JobState::Running)
            .unwrap();
        m.journal()
            .append_event(
                &rec.id,
                "probe",
                &serde_json::json!({ "resumable": true, "session_token": "mock-session" }),
            )
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

    // simulate the crash window: chunk leased, half confirmed, confirm lost.
    // The journaled probe event is part of that window (see #12).
    m.journal()
        .set_job_state(&rec.id, JobState::Running)
        .unwrap();
    m.journal()
        .append_event(
            &rec.id,
            "probe",
            &serde_json::json!({ "resumable": true, "session_token": "mock-session" }),
        )
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
        probe_cap: None,
        probe_max_parallel: None,
        log: Mutex::new(Vec::new()),
        has_session: std::sync::atomic::AtomicBool::new(false),
        fail_session_at_call: AtomicU32::new(0),
        fail_session_forever: std::sync::atomic::AtomicBool::new(false),
        reattach_count: AtomicU32::new(0),
        in_flight: AtomicUsize::new(0),
        max_in_flight: AtomicUsize::new(0),
        wedge_idx: AtomicU32::new(u32::MAX),
        wedge_release: Arc::new(tokio::sync::Notify::new()),
        wedge_engaged: Arc::new(AtomicBool::new(false)),
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

// ---------------------------------------------------------------------------
// Sprint 1 — concurrent supervisor, progress telemetry, slow-chunk eviction
// ---------------------------------------------------------------------------

/// Scripted per-chunk behavior for the concurrency/eviction drills.
enum Behavior {
    /// First call never returns (no progress, no error) — eviction fodder.
    /// Later calls complete instantly.
    StallOnce,
    /// Stream `ticks × tick_bytes` through the progress channel, one byte
    /// batch every `tick_ms`, then confirm the whole segment.
    Healthy {
        ticks: u32,
        tick_bytes: u64,
        tick_ms: u64,
    },
    /// Immediate unrecoverable error (spec: no auto-retry).
    Fatal,
    /// Sleep, then confirm the whole segment.
    SlowMs(u64),
}

/// Adapter with a per-idx script + per-idx call counter, so a test can say
/// "chunk 1 stalls the first time it is leased and succeeds the second".
struct ScriptedAdapter {
    cap: Capability,
    behaviors: Vec<Behavior>,
    calls: Mutex<std::collections::HashMap<u32, u32>>,
    log: Mutex<Vec<UploadLog>>,
}

impl ScriptedAdapter {
    fn new(cap: Capability, behaviors: Vec<Behavior>) -> Self {
        Self {
            cap,
            behaviors,
            calls: Mutex::new(std::collections::HashMap::new()),
            log: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl TransferAdapter for ScriptedAdapter {
    fn name(&self) -> &'static str {
        "mock_scripted"
    }
    fn capabilities(&self) -> Capability {
        self.cap
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: self.cap.has(Capability::RESUMABLE),
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
            session_token: Some("mock-scripted".into()),
        })
    }
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            let c = calls.entry(lease.idx).or_insert(0);
            *c += 1;
            *c
        };
        // Log at ENTRY: a call that never completes (stall, fatal, evicted
        // mid-body) must still be visible to tests. `bytes_moved` is patched
        // with the real figure at completion, below.
        let entry = {
            let mut log = self.log.lock().unwrap();
            log.push(UploadLog {
                idx: lease.idx,
                resume_from: lease.resume_from,
                bytes_moved: 0,
            });
            log.len() - 1
        };
        let behavior = &self.behaviors[lease.idx as usize];
        match behavior {
            Behavior::StallOnce if call == 1 => {
                // connected but wedged: zero progress, zero response
                std::future::pending::<()>().await;
                unreachable!()
            }
            Behavior::Healthy {
                ticks,
                tick_bytes,
                tick_ms,
            } => {
                for _ in 0..*ticks {
                    progress.add(*tick_bytes);
                    tokio::time::sleep(std::time::Duration::from_millis(*tick_ms)).await;
                }
            }
            Behavior::Fatal => {
                return Err(AdapterError::Unresumable("mock fatal".into()));
            }
            Behavior::SlowMs(ms) => {
                tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
            }
            _ => {}
        }

        // Read the real slice (same discipline as MockAdapter) and confirm it all.
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&spec.source_path)
            .map_err(|e| AdapterError::Transient(format!("source vanished: {e}")))?;
        file.seek(SeekFrom::Start(lease.resume_from)).unwrap();
        let want = (lease.end - lease.resume_from) as usize;
        let mut buf = vec![0u8; want];
        file.read_exact(&mut buf)
            .map_err(|e| AdapterError::Transient(format!("short read: {e}")))?;
        for (i, b) in buf.iter().enumerate() {
            assert_eq!(*b, pattern_byte(lease.resume_from + i as u64));
        }
        if !matches!(behavior, Behavior::Healthy { .. }) {
            progress.add(want as u64);
        }
        self.log.lock().unwrap()[entry].bytes_moved = want as u64;
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: lease.end - lease.resume_from,
            adapter_token: Some(format!("tok-{call}-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("mock scripted ok".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        // Test double: no real session map to restore (issue #12).
        Ok(())
    }
}

fn scripted_manager(
    adapter: Arc<ScriptedAdapter>,
    eviction: ium_core::EvictionPolicy,
) -> JobManager {
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(adapter);
    m.set_eviction_policy(eviction);
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 8,
        backoff_base_ms: 1,
        backoff_cap_ms: 5,
        respect_retry_after: true,
    });
    m
}

#[tokio::test]
async fn concurrent_workers_upload_each_chunk_exactly_once() {
    // 4 chunks, 4 workers, all in flight together. The journal must show
    // exactly one upload call per chunk, each at its own offset, and every
    // chunk verified — no double-send, no skipped chunk, no interleaving bug.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "conc.bin", 20 * MIB);

    let mock = Arc::new(MockAdapter {
        cap: Capability::new(Capability::RESUMABLE),
        fail_first: AtomicU32::new(0),
        partial_first_len: None,
        chunk_delay_ms: 15,
        probe_cap: None,
        probe_max_parallel: None,
        log: Mutex::new(Vec::new()),
        has_session: std::sync::atomic::AtomicBool::new(false),
        fail_session_at_call: AtomicU32::new(0),
        fail_session_forever: std::sync::atomic::AtomicBool::new(false),
        reattach_count: AtomicU32::new(0),
        in_flight: AtomicUsize::new(0),
        max_in_flight: AtomicUsize::new(0),
        wedge_idx: AtomicU32::new(u32::MAX),
        wedge_release: Arc::new(tokio::sync::Notify::new()),
        wedge_engaged: Arc::new(AtomicBool::new(false)),
    });
    let m = manager_with(Arc::clone(&mock));
    let rec = m.create_job(&spec_for(&path, 20 * MIB), 4).unwrap();

    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, JobState::Done);

    let log = mock.log.lock().unwrap().clone();
    assert_eq!(log.len(), 4, "exactly one call per chunk: {log:?}");
    let mut seen: Vec<u32> = log.iter().map(|l| l.idx).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4, "each chunk uploaded once, not twice");

    let snap = m.snapshot(&rec.id).unwrap();
    for (i, c) in snap.chunks.iter().enumerate() {
        assert_eq!(c.state, ium_core::chunk::ChunkState::Verified, "chunk {i}");
        assert_eq!(c.bytes_confirmed, c.end - c.begin, "chunk {i} fully acked");
    }
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        20 * MIB
    );
}

#[tokio::test]
async fn stalled_chunk_is_evicted_and_job_still_completes() {
    // Spec §4.2: a chunk making no progress while a peer streams normally is
    // evicted after the strike budget; the chunk is re-leased and resumes —
    // the job completes instead of hanging on the wedged connection.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "stall.bin", 10 * MIB);

    let mock = Arc::new(ScriptedAdapter::new(
        Capability::new(Capability::RESUMABLE),
        vec![
            // healthy peer: ~500 ms of steady 256 KB/20ms streaming
            Behavior::Healthy {
                ticks: 25,
                tick_bytes: 256_000,
                tick_ms: 20,
            },
            Behavior::StallOnce,
        ],
    ));
    let m = scripted_manager(
        Arc::clone(&mock),
        ium_core::EvictionPolicy {
            window: std::time::Duration::from_millis(40),
            floor_ratio: 0.4,
            strikes_to_evict: 2,
        },
    );
    let rec = m
        .create_job(&scripted_spec_for(&path, 10 * MIB), 2)
        .unwrap();
    assert_eq!(m.snapshot(&rec.id).unwrap().chunks.len(), 2);

    let started = std::time::Instant::now();
    let done = m.run_job(&rec.id).await.unwrap();
    let elapsed = started.elapsed();
    assert_eq!(done.state, JobState::Done);

    // Eviction had to happen (the stall was infinite) and promptly.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "eviction must rescue the job, took {elapsed:?}"
    );
    let snap = m.snapshot(&rec.id).unwrap();
    let evictions: Vec<&(i64, String, String)> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_evicted")
        .collect();
    assert!(!evictions.is_empty(), "the stalled chunk was evicted");
    let evicted_idx = serde_json::from_str::<serde_json::Value>(&evictions[0].2).unwrap()["idx"]
        .as_u64()
        .unwrap();
    assert_eq!(evicted_idx, 1, "chunk 1 was the stalled one");

    // The stalled lease kept nothing (no server ack while wedged), so the
    // re-lease had to start from chunk 1's begin. Lease.resume_from is the
    // absolute file offset, and the wedged call itself moved zero bytes.
    let log = mock.log.lock().unwrap().clone();
    let idx1_calls: Vec<&UploadLog> = log.iter().filter(|l| l.idx == 1).collect();
    assert_eq!(idx1_calls.len(), 2, "stalled once, re-leased once: {log:?}");
    let chunk1_begin = snap.chunks[1].begin;
    assert_eq!(
        idx1_calls[0].resume_from, chunk1_begin,
        "wedged at chunk begin"
    );
    assert_eq!(
        idx1_calls[0].bytes_moved, 0,
        "the wedged call moved nothing"
    );
    assert_eq!(
        idx1_calls[1].resume_from, chunk1_begin,
        "re-lease resumed from chunk begin — no server-confirmed bytes to resume from"
    );
    assert_eq!(
        idx1_calls[1].bytes_moved,
        5 * MIB,
        "re-lease completed the chunk"
    );

    // Both chunks fully confirmed and verified at the end.
    for c in &snap.chunks {
        assert_eq!(c.state, ium_core::chunk::ChunkState::Verified);
        assert_eq!(c.bytes_confirmed, c.end - c.begin);
    }
}

#[tokio::test]
async fn fatal_error_stops_sibling_workers_promptly() {
    // One chunk hits an unrecoverable error while its peers are mid-flight
    // inside 10-second uploads. The fleet abort must cut the siblings loose
    // (dropping their in-flight requests) and journal Failed — in seconds,
    // not after the 10 s sleeps.
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "fatal.bin", 20 * MIB);

    let mock = Arc::new(ScriptedAdapter::new(
        Capability::new(Capability::RESUMABLE),
        vec![
            Behavior::Fatal,
            Behavior::SlowMs(10_000),
            Behavior::SlowMs(10_000),
            Behavior::SlowMs(10_000),
        ],
    ));
    let m = scripted_manager(
        Arc::clone(&mock),
        ium_core::EvictionPolicy {
            window: std::time::Duration::from_secs(10), // irrelevant here
            floor_ratio: 0.4,
            strikes_to_evict: 2,
        },
    );
    let rec = m
        .create_job(&scripted_spec_for(&path, 20 * MIB), 4)
        .unwrap();

    let started = std::time::Instant::now();
    let out = tokio::time::timeout(std::time::Duration::from_secs(5), m.run_job(&rec.id))
        .await
        .expect("fleet abort must not hang on sibling uploads")
        .unwrap();
    assert_eq!(out.state, JobState::Failed);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "abort was prompt: {:?}",
        started.elapsed()
    );

    let snap = m.snapshot(&rec.id).unwrap();
    // The fatal chunk: journaled back to Pending (retry budget exhausted by
    // an unretryable error class).
    let fatal = snap.chunks.iter().find(|c| c.idx == 0).unwrap();
    assert_eq!(fatal.state, ium_core::chunk::ChunkState::Pending);
    assert_eq!(fatal.bytes_confirmed, 0);
    assert!(snap
        .events
        .iter()
        .any(|(_, k, p)| k == "chunk_error" && p.contains("Unresumable")));
    // The aborted siblings stay `Leased` in the journal — their in-flight
    // requests were cancelled mid-body, state unknown, exactly like a crash.
    // A later recovery path reclaims them (see stale_lease test).
    for idx in 1..4 {
        let c = snap.chunks.iter().find(|c| c.idx == idx).unwrap();
        assert_eq!(
            c.state,
            ium_core::chunk::ChunkState::Leased,
            "aborted sibling {idx} left recoverable"
        );
    }
}
