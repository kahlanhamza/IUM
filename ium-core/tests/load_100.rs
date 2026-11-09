//! Lane 5 (#66): product scalability — the supervisor concurrency limiter
//! and the 100-jobs load e2e.
//!
//! RED at current HEAD: `Supervisor::with_concurrency_limit` /
//! `with_handle_and_limit` do not exist — the supervisor spawns one drive
//! task per `drive()` call with NO cap (W2), so a 100-file drag-drop intake
//! is 100 simultaneous uploads. These tests define the contract that fixes
//! it, then drive the real failure-path invariant at scale: every one of
//! 100 interleaved jobs ends in Done or Failed, retries journaled, bounds
//! held (the soak's honesty pattern, #34).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::retry::RetryPolicy;
use ium_core::state::JobState;
use ium_core::supervisor::Supervisor;
use ium_core::{JobSpec, SessionJournal};

const KIB: u64 = 1024;

/// Test-speed retry policy: the default's 500 ms → 30 s backoff would turn
/// 100 jobs × flaky chunks into a minutes-long test; 1 ms keeps the
/// semantics (attempts counted, backoff honored) at CI speed.
fn fast_policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 8,
        backoff_base_ms: 1,
        backoff_cap_ms: 5,
        respect_retry_after: false,
    }
}

/// Per-endpoint behavior, keyed by the JobSpec's endpoint (one adapter for
/// every job in the process — the registry shape the shell uses too).
#[derive(Clone)]
enum Behavior {
    /// Ack the whole lease on the first call.
    Clean,
    /// Fail `k` calls with a Transient error, then ack — the wire shape of
    /// a blip the retry policy must absorb (retries counted, job Done).
    Flaky(u32),
    /// Fail every call — the attempt budget must exhaust into an honest
    /// Failed, never an infinite loop.
    Dead,
    /// Park every call until the test releases the endpoint's notify —
    /// holds drive slots deterministically for the limiter tests.
    Wedge(Arc<tokio::sync::Notify>),
}

struct LoadAdapter {
    behaviors: HashMap<String, Behavior>,
    /// endpoint → failures still owed (Flaky's deterministic countdown).
    remaining: Mutex<HashMap<String, u32>>,
    /// Wedge entries currently parked (shared with the tests, which poll it
    /// before releasing — a spawned task's internals are not reachable).
    engaged: Arc<AtomicU32>,
    /// Total upload_chunk calls (the load summary's work proof).
    calls: AtomicU32,
}

impl LoadAdapter {
    /// Build the adapter; also hands back the shared wedge counter so the
    /// tests can poll "is anything parked?" without touching the manager's
    /// registry internals.
    fn new(behaviors: HashMap<String, Behavior>) -> (Self, Arc<AtomicU32>) {
        let engaged = Arc::new(AtomicU32::new(0));
        (
            Self {
                behaviors,
                remaining: Mutex::new(HashMap::new()),
                engaged: Arc::clone(&engaged),
                calls: AtomicU32::new(0),
            },
            engaged,
        )
    }

    fn behavior_for(&self, endpoint: &str) -> Behavior {
        self.behaviors
            .get(endpoint)
            .cloned()
            .unwrap_or(Behavior::Clean)
    }
}

#[async_trait::async_trait]
impl TransferAdapter for LoadAdapter {
    fn name(&self) -> &'static str {
        "load"
    }

    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }

    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
            uniform_block: None,
            session_token: None,
            concatenation: false,
        })
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.behavior_for(&spec.endpoint) {
            Behavior::Wedge(notify) => {
                self.engaged.fetch_add(1, Ordering::SeqCst);
                notify.notified().await;
                self.engaged.fetch_sub(1, Ordering::SeqCst);
            }
            Behavior::Flaky(k) => {
                let mut map = self.remaining.lock().unwrap();
                let left = map.entry(spec.endpoint.clone()).or_insert(k);
                if *left > 0 {
                    *left -= 1;
                    drop(map);
                    return Err(AdapterError::Transient(format!(
                        "load flaky blip: endpoint {}",
                        spec.endpoint
                    )));
                }
            }
            Behavior::Dead => {
                return Err(AdapterError::Transient(format!(
                    "load dead endpoint: {}",
                    spec.endpoint
                )));
            }
            Behavior::Clean => {}
        }
        let span = lease.end - lease.resume_from;
        progress.add(span);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: None,
        })
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let ok = std::fs::metadata(&spec.source_path)
            .map(|m| m.len() == spec.source_size)
            .unwrap_or(false);
        Ok(VerifyReport {
            ok,
            detail: Some(format!(
                "load verify: {} calls so far, size {} == declared",
                self.calls.load(Ordering::Relaxed),
                spec.source_size
            )),
        })
    }

    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn make_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    // Cheap distinct-ish bytes (the soak's pattern); the content is not the
    // subject — the 100 interleaved state machines are.
    let mut v = Vec::with_capacity(size as usize);
    for off in 0..size {
        v.push(((off + size) % 251) as u8);
    }
    std::fs::write(&path, &v).unwrap();
    path
}

fn spec_for(path: &std::path::Path, size: u64, endpoint: &str) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "load".into(),
        endpoint: endpoint.into(),
    }
}

fn rss_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
}

fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|d| d.count())
        .unwrap_or(0)
}

fn is_terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Done | JobState::Failed | JobState::Unresumable | JobState::Canceled
    )
}

fn manager_with(adapter: LoadAdapter) -> (Arc<JobManager>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let journal = SessionJournal::open(tmp.path().join("journal.db")).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.set_policy(fast_policy());
    m.register_adapter(Arc::new(adapter));
    (Arc::new(m), tmp)
}

/// Sizes cycle so the 100 jobs do NOT share one shape (all stay under the
/// 5 MiB CHUNK_MIN → one chunk each — wedge tests stay deterministic).
fn size_for(idx: usize) -> u64 {
    [128 * KIB, 256 * KIB, 512 * KIB, 1024 * KIB][idx % 4]
}

// ---------------------------------------------------------------- limiter

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn limit_admits_only_max_and_pops_fifo() {
    let notify = Arc::new(tokio::sync::Notify::new());
    let behaviors: HashMap<String, Behavior> = (0..5)
        .map(|i| {
            (
                format!("load://j{i}/wedge"),
                Behavior::Wedge(notify.clone()),
            )
        })
        .collect();
    let (adapter, _engaged) = LoadAdapter::new(behaviors);
    let (m, tmp) = manager_with(adapter);
    let sup = Supervisor::with_concurrency_limit(Arc::clone(&m), 2);

    let mut ids = Vec::new();
    for i in 0..5 {
        let p = make_source(tmp.path(), &format!("w{i}.bin"), size_for(i));
        let rec = m
            .create_job(&spec_for(&p, size_for(i), &format!("load://j{i}/wedge")), 2)
            .unwrap();
        ids.push(rec.id.clone());
        sup.drive(&rec.id);
    }

    // The first two drive calls hold the two slots; the rest are queued
    // (is_driving stays honest: queued ≠ driving).
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(sup.is_driving(&ids[0]) && sup.is_driving(&ids[1])) {
        assert!(Instant::now() < deadline, "limiter never admitted 2");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!sup.is_driving(&ids[2]) && !sup.is_driving(&ids[3]) && !sup.is_driving(&ids[4]));

    // Release ONE wedge → exactly one queued job pops, FIFO (job 2). Every
    // wedge-test file is one chunk (≤ 1 MiB < CHUNK_MIN), so one wake is
    // exactly one slot freed.
    notify.notify_one();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !sup.is_driving(&ids[2]) {
        assert!(Instant::now() < deadline, "FIFO pop never happened");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!sup.is_driving(&ids[3]) && !sup.is_driving(&ids[4]));

    // Release everything → every job drains to Done (the drain pattern is
    // re-arm-safe: a stored permit wakes the next parker if none waits).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let states: Vec<JobState> = ids
            .iter()
            .map(|id| m.snapshot(id).unwrap().job.state)
            .collect();
        if states.iter().all(|s| *s == JobState::Done) {
            break;
        }
        notify.notify_one();
        assert!(Instant::now() < deadline, "queue never drained: {states:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_job_canceled_is_never_driven() {
    let notify = Arc::new(tokio::sync::Notify::new());
    let behaviors: HashMap<String, Behavior> = [
        (
            "load://a/wedge".to_string(),
            Behavior::Wedge(notify.clone()),
        ),
        ("load://b/queued".to_string(), Behavior::Clean),
    ]
    .into_iter()
    .collect();
    let (adapter, _engaged) = LoadAdapter::new(behaviors);
    let (m, tmp) = manager_with(adapter);
    let sup = Supervisor::with_concurrency_limit(Arc::clone(&m), 1);

    let pa = make_source(tmp.path(), "a.bin", 256 * KIB);
    let pb = make_source(tmp.path(), "b.bin", 256 * KIB);
    let ra = m
        .create_job(&spec_for(&pa, 256 * KIB, "load://a/wedge"), 2)
        .unwrap();
    let rb = m
        .create_job(&spec_for(&pb, 256 * KIB, "load://b/queued"), 2)
        .unwrap();
    sup.drive(&ra.id);
    sup.drive(&rb.id); // queued — the single slot is held by a's wedge

    // Cancel while queued: the manager journals Canceled (Created is
    // cancellable); no drive task exists to abort.
    let canceled = sup.cancel(&rb.id).await.unwrap();
    assert_eq!(canceled.state, JobState::Canceled);

    // Release the wedge → a completes; b's stale queue entry must be
    // dropped WITHOUT driving (no probe event — it never contacted an
    // endpoint).
    notify.notify_one();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if m.snapshot(&ra.id).unwrap().job.state == JobState::Done {
            break;
        }
        assert!(Instant::now() < deadline, "a never finished");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(m.snapshot(&rb.id).unwrap().job.state, JobState::Canceled);
    let events = m.journal().events_for(&rb.id).unwrap();
    assert!(
        !events.iter().any(|(_, k, _)| k == "probe"),
        "a canceled-while-queued job must never drive: {events:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_signal_reenters_through_the_limiter() {
    let notify = Arc::new(tokio::sync::Notify::new());
    let behaviors: HashMap<String, Behavior> = [(
        "load://p/wedge".to_string(),
        Behavior::Wedge(notify.clone()),
    )]
    .into_iter()
    .collect();
    let (adapter, engaged) = LoadAdapter::new(behaviors);
    let (m, tmp) = manager_with(adapter);
    let sup = Supervisor::with_concurrency_limit(Arc::clone(&m), 1);

    let p = make_source(tmp.path(), "p.bin", 512 * KIB);
    let rec = m
        .create_job(&spec_for(&p, 512 * KIB, "load://p/wedge"), 2)
        .unwrap();
    sup.drive(&rec.id);

    // Wedge engaged → the slot is held mid-upload.
    let deadline = Instant::now() + Duration::from_secs(10);
    while engaged.load(Ordering::SeqCst) < 1 {
        assert!(Instant::now() < deadline, "wedge never engaged");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Cooperative pause takes effect between leases: release the parked
    // lease (drain-pattern: keep waking until the state lands), the task
    // exits and the slot frees with the job parked in Paused.
    sup.pause(&rec.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if m.snapshot(&rec.id).unwrap().job.state == JobState::Paused {
            break;
        }
        notify.notify_one();
        assert!(Instant::now() < deadline, "pause never landed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Resume: re-drives THROUGH the limiter (a free slot → immediate run).
    sup.resume_signal(&rec.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if m.snapshot(&rec.id).unwrap().job.state == JobState::Done {
            break;
        }
        notify.notify_one();
        assert!(Instant::now() < deadline, "resumed job never finished");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[test]
fn limit_zero_is_an_honest_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let journal = SessionJournal::open(tmp.path().join("j.db")).unwrap();
    let m = Arc::new(JobManager::with_journal(journal).unwrap());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _ = Supervisor::with_concurrency_limit(m, 0);
    }));
    assert!(result.is_err(), "max = 0 must panic, not silently unbound");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unbounded_construction_is_unchanged() {
    let notify = Arc::new(tokio::sync::Notify::new());
    let behaviors: HashMap<String, Behavior> = (0..3)
        .map(|i| {
            (
                format!("load://u{i}/wedge"),
                Behavior::Wedge(notify.clone()),
            )
        })
        .collect();
    let (adapter, engaged) = LoadAdapter::new(behaviors);
    let (m, tmp) = manager_with(adapter);
    // `new` stays UNBOUNDED (D1): three wedges, three simultaneous drives.
    let sup = Supervisor::new(Arc::clone(&m));
    let mut ids = Vec::new();
    for i in 0..3 {
        let p = make_source(tmp.path(), &format!("u{i}.bin"), size_for(i));
        let rec = m
            .create_job(&spec_for(&p, size_for(i), &format!("load://u{i}/wedge")), 2)
            .unwrap();
        ids.push(rec.id.clone());
        sup.drive(&rec.id);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while engaged.load(Ordering::SeqCst) < 3 {
        assert!(
            Instant::now() < deadline,
            "unbounded supervisor must admit all 3"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let states: Vec<JobState> = ids
            .iter()
            .map(|id| m.snapshot(id).unwrap().job.state)
            .collect();
        if states.iter().all(|s| *s == JobState::Done) {
            break;
        }
        notify.notify_one();
        assert!(
            Instant::now() < deadline,
            "wedges never released: {states:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ------------------------------------------------------------ the load e2e

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn load_100_jobs_reach_honest_terminals_through_the_limiter() {
    // 100 jobs, ONE manager, ONE journal, limit 8:
    //   idx 0..=3    clean       → Done (4)
    //   idx 4..=91   flaky k     → Done with journaled retries (88)
    //   idx 92..=99  dead        → Failed after the attempt budget (8)
    let mut behaviors = HashMap::new();
    for i in 0..100 {
        let endpoint = format!("load://job{i}");
        behaviors.insert(
            endpoint,
            match i {
                0..=3 => Behavior::Clean,
                4..=91 => Behavior::Flaky((i % 3) as u32),
                _ => Behavior::Dead,
            },
        );
    }
    let (adapter, _engaged) = LoadAdapter::new(behaviors);

    let rss_before = rss_kb();
    let fd_before = fd_count();
    let t0 = Instant::now();

    let (m, tmp) = manager_with(adapter);
    let sup = Supervisor::with_concurrency_limit(Arc::clone(&m), 8);

    let mut ids = Vec::with_capacity(100);
    for i in 0..100 {
        let size = size_for(i);
        let p = make_source(tmp.path(), &format!("src{i}.bin"), size);
        let rec = m
            .create_job(&spec_for(&p, size, &format!("load://job{i}")), 4)
            .unwrap();
        ids.push(rec.id.clone());
        sup.drive(&rec.id);
    }

    // Journal-truth polling, exactly what the UI's watcher does — at 100×.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let states: Vec<JobState> = ids
            .iter()
            .map(|id| m.snapshot(id).unwrap().job.state)
            .collect();
        if states.iter().all(|s| is_terminal(*s)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "100 jobs did not reach terminals in time: {} non-terminal",
            states.iter().filter(|s| !is_terminal(**s)).count()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let elapsed = t0.elapsed();

    // The exact honest mix — not "some terminals": 92 Done, 8 Failed, zero
    // stuck, zero canceled, zero unresumable.
    let states: Vec<JobState> = ids
        .iter()
        .map(|id| m.snapshot(id).unwrap().job.state)
        .collect();
    let done = states.iter().filter(|s| **s == JobState::Done).count();
    let failed = states.iter().filter(|s| **s == JobState::Failed).count();
    assert_eq!(done, 92, "clean+flaky must all be Done: {states:?}");
    assert_eq!(failed, 8, "dead endpoints must all be Failed: {states:?}");

    // The failure path kept its receipts: every flaky job's journal carries
    // chunk_error events (the retry evidence), and the dead ones exhausted
    // their budget with the same receipts.
    let mut flaky_with_receipts = 0usize;
    let mut dead_receipt_min = u64::MAX;
    for (i, id) in ids.iter().enumerate() {
        let events = m.journal().events_for(id).unwrap();
        let n_errors = events.iter().filter(|(_, k, _)| k == "chunk_error").count();
        match i {
            4..=91 => {
                let owed = (i % 3) as u64;
                assert!(
                    n_errors as u64 >= owed,
                    "job {i} owed {owed} retries, journal has {n_errors}"
                );
                if n_errors > 0 {
                    flaky_with_receipts += 1;
                }
            }
            92..=99 => {
                assert!(
                    n_errors >= 8,
                    "dead job {i} exhausted nothing: {n_errors} errors"
                );
                dead_receipt_min = dead_receipt_min.min(n_errors as u64);
            }
            _ => {
                assert_eq!(n_errors, 0, "clean job {i} must have a clean journal");
            }
        }
    }
    assert!(
        flaky_with_receipts > 0,
        "the flaky population must prove retries"
    );

    // Bounds: the orchestration of 100 jobs is bounded by the PLAN (the
    // limit), not by the job count.
    let rss_after = rss_kb();
    let rss_delta_kb = rss_after.zip(rss_before).map(|(a, b)| a.saturating_sub(b));
    let fd_delta = fd_count().saturating_sub(fd_before);
    assert!(
        fd_delta < 400,
        "fd leak: {fd_delta} new descriptors for 100 jobs"
    );
    if let Some(delta) = rss_delta_kb {
        // Under the sanitizer lane (issue #95 R4) ASan's own footprint
        // dwarfs the engine's — the absolute RSS budget belongs to
        // production builds; there the pin skips loud, and LSan owns the
        // leak finding at process exit.
        if std::env::var_os("ASAN_OPTIONS").is_some() {
            eprintln!(
                "skipped under sanitizers: {delta} KiB is instrumentation \
                 footprint, not orchestration state"
            );
        } else {
            assert!(
                delta < 512 * 1024,
                "RSS grew {delta} KiB for 100 small jobs — unbounded orchestration"
            );
        }
    }

    // The summary for the issue record (the soak's honesty pattern).
    let summary = serde_json::json!({
        "jobs": 100,
        "limit": 8,
        "done": done,
        "failed": failed,
        "flaky_with_retry_receipts": flaky_with_receipts,
        "dead_min_chunk_errors": dead_receipt_min,
        "elapsed_secs": elapsed.as_secs_f64(),
        "fd_delta": fd_delta,
        "rss_delta_kb": rss_delta_kb,
        "bounds": { "secs": 120, "fd": 400, "rss_kib": 512 * 1024 },
    });
    println!("LOAD-100 {summary}");
}
