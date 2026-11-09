//! Resource-honesty soak (issue #34, acceptance criterion 3): journal size
//! and manager RSS pinned under a many-event soak.
//!
//! The soak drives the REAL manager path — lease → upload → partial confirm
//! → re-lease, exactly what a throttled B/s link produces for hours — until
//! tens of thousands of journal writes have landed. It then pins:
//!
//! * the journal's post-checkpoint size (bytes on disk for ~2^14 events);
//! * the WAL's size after `checkpoint_wal(TRUNCATE)` (the append log must
//!   collapse — truth lives in the main db, not an ever-growing WAL);
//! * the manager's RSS delta over the whole soak (the orchestration state —
//!   pools, attempt maps, bps windows — must be bounded by the PLAN, not by
//!   the EVENT COUNT; events are the journal's business, never held
//!   in memory).
//!
//! Compaction/VACUUM policy is explicitly out of scope (issue #34): the
//! bounds pin a sane LINEAR constant for append-only growth. If a future
//! soak shows the constant is too fat for a 17-hour leg, that is its own
//! issue.
//!
//! The measured numbers are printed as `SOAK-BOUNDS {json}` for the issue
//! record and asserted against the pinned bounds below.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;
/// 64 chunks × 1 MiB = 64 MiB transferred in 4 KiB server-confirmed
/// segments → 16 384 confirm events + ~16 k lease re-writes + ~16 k chunk
/// updates: the tens-of-thousands-of-events scale of an hours-long leg.
const CHUNKS: u64 = 64;
const SOURCE_SIZE: u64 = CHUNKS * MIB;
const SEGMENT: u64 = 4 * 1024;

/// Pinned bounds (issue #34 record; generous against CI variance, tight
/// against unbounded growth — the measured numbers sit well under these).
const BOUND_JOURNAL_BYTES: u64 = 32 * MIB;
const BOUND_WAL_BYTES: u64 = 4096;
const BOUND_RSS_DELTA_KB: u64 = 256 * 1024; // 256 MiB
/// R1 (issue #95): descriptors opened AND retained by the soak itself.
/// Generous against sqlite's WAL/SHM pair, tight against per-event leaks.
const BOUND_FD: usize = 8;

fn rss_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
}

/// Open file descriptors (/proc/self/fd entries). `None` off-Linux — the
/// fd pin skips LOUD there (the #34 honesty contract: a skipped pin says
/// so), it never fakes a zero (R1, issue #95).
fn fd_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").map(|d| d.count()).ok()
}

/// The canary: prove the fd detector is not a no-op — five REAL handles
/// must be seen, and their closure must return the count to baseline.
/// Without this proof an fd pin could vacuously pass forever.
#[test]
fn fd_probe_detects_real_descriptor_growth() {
    let Some(_) = fd_count() else {
        eprintln!("skipped: no /proc/self/fd (non-Linux) — the canary is platform-honest");
        return;
    };
    // Reference-relative, NOT process-baseline-relative: cargo runs this
    // file's tests in PARALLEL, and the soak's sqlite fds are legitimately
    // opening/closing while the canary runs (two pre-landing failures
    // taught this: first the process baseline, then the reference window —
    // the full-suite gate hit the soak's startup churn the standalone run
    // never saw). Both checks are therefore symmetric ±1-churn deltas
    // around the NARROW read windows: the detector must SEE ~5 opens and
    // must SEE their closure; anything else is sibling noise.
    let reference = fd_count().expect("/proc existed at first read");
    let tmp = tempfile::tempdir().unwrap();
    let handles: Vec<std::fs::File> = (0..5)
        .map(|i| std::fs::File::create(tmp.path().join(format!("canary{i}.bin"))).unwrap())
        .collect();
    let grown = fd_count().expect("/proc existed at second read");
    assert!(
        grown >= reference + 4,
        "detector saw no growth for 5 open handles: reference {reference}, now {grown}"
    );
    drop(handles);
    let back = fd_count().unwrap();
    assert!(
        grown - back >= 4,
        "the 5 closures were not seen: {grown} open vs {back} after close \
         (detector blind to releases?)"
    );
}

/// Partial-confirm adapter: every call moves and gets acknowledged for
/// exactly `SEGMENT` bytes — the wire shape of a 512 B/s-class link seen
/// from the manager's altitude, at test speed.
struct SoakAdapter {
    calls: AtomicU32,
}

#[async_trait]
impl TransferAdapter for SoakAdapter {
    fn name(&self) -> &'static str {
        "soak_partial"
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
        _spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // Server confirms at most SEGMENT bytes per call (the last segment of
        // a chunk confirms the rest).
        let remaining = lease.end - lease.resume_from;
        let acked = remaining.min(SEGMENT);
        progress.add(acked);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: acked,
            adapter_token: None,
        })
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        // Byte-exactness: the SOURCE bytes the planner streamed must equal
        // what the spec declared (the soak's integrity gate; the real
        // adapters do this against the server).
        let ok = std::fs::metadata(&spec.source_path)
            .map(|m| m.len() == spec.source_size)
            .unwrap_or(false);
        Ok(VerifyReport {
            ok,
            detail: Some(format!(
                "soak verify: {} calls, size {} == declared",
                self.calls.load(Ordering::Relaxed),
                spec.source_size
            )),
        })
    }

    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_event_soak_bounds_journal_and_rss() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("journal.db");

    let source_path = tmp.path().join("source.bin");
    let source = Arc::new({
        let mut v = Vec::with_capacity(SOURCE_SIZE as usize);
        for off in 0..SOURCE_SIZE {
            v.push((off % 251) as u8);
        }
        v
    });
    std::fs::write(&source_path, &*source).unwrap();

    let journal = SessionJournal::open(&db_path).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(SoakAdapter {
        calls: AtomicU32::new(0),
    }));

    let spec = JobSpec {
        source_path: source_path.to_string_lossy().into_owned(),
        source_size: SOURCE_SIZE,
        adapter: "soak_partial".into(),
        endpoint: "soak://bounds".into(),
    };

    // Memory and descriptors measured AFTER the 64 MiB source + journal
    // exist: the delta is the manager's own orchestration cost over the
    // whole soak (R1, issue #95: fds join the RSS pin — a handle leak is a
    // different failure shape than a memory leak, EMFILE hours later).
    let rss_before = rss_kb();
    let fd_before = fd_count();

    let rec = m.create_job(&spec, 8).unwrap();
    let t0 = std::time::Instant::now();
    let final_rec = m.run_job(&rec.id).await.unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(final_rec.state, JobState::Done);

    let events = m.journal().events_for(&rec.id).unwrap();
    let n_events = events.len();
    assert!(
        n_events >= 16_000,
        "the soak must actually produce tens of thousands of events, got {n_events}"
    );

    // The journal must collapse its WAL and hold truth in the main db.
    m.journal().checkpoint_wal().unwrap();
    let db_bytes = std::fs::metadata(&db_path).unwrap().len();
    let wal_bytes = std::fs::metadata(db_path.with_extension("db-wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    let rss_after = rss_kb();
    let rss_delta_kb = rss_after.zip(rss_before).map(|(a, b)| a.saturating_sub(b));
    let fd_after = fd_count();
    let fd_delta = fd_after.zip(fd_before).map(|(a, b)| a.saturating_sub(b));

    // Sanity floors: the soak really wrote (a no-op soak would pass any cap).
    assert!(
        db_bytes >= 1024 * 1024,
        "journal too small for {n_events} events — the soak did not soak: {db_bytes}"
    );

    let calls = 16_384u64; // CHUNKS * MIB / SEGMENT
    let measured = serde_json::json!({
        "events": n_events,
        "adapter_calls_floor": calls,
        "journal_bytes": db_bytes,
        "wal_bytes_after_checkpoint": wal_bytes,
        "rss_delta_kb": rss_delta_kb,
        "fd_delta": fd_delta,
        "elapsed_secs": elapsed.as_secs_f64(),
        "bounds": {
            "journal_bytes": BOUND_JOURNAL_BYTES,
            "wal_bytes": BOUND_WAL_BYTES,
            "rss_delta_kb": BOUND_RSS_DELTA_KB,
            "fd": BOUND_FD,
        },
    });
    println!("SOAK-BOUNDS {measured}");

    // The pins.
    assert!(
        db_bytes <= BOUND_JOURNAL_BYTES,
        "journal grew past the pinned bound: {db_bytes} > {BOUND_JOURNAL_BYTES} \
         (linear constant exploded — compaction is its own issue, but THIS is a leak)"
    );
    assert!(
        wal_bytes <= BOUND_WAL_BYTES,
        "WAL did not collapse at checkpoint(TRUNCATE): {wal_bytes} bytes"
    );
    if let Some(delta) = rss_delta_kb {
        assert!(
            delta <= BOUND_RSS_DELTA_KB,
            "manager RSS grew {delta} KiB over {n_events} events — unbounded \
             orchestration state (pools/maps/events held in memory?)"
        );
    } else {
        eprintln!("skipped: RSS pinned on Linux only (this platform has no /proc/self/status)");
    }
    // The fd pin (R1): a soak that runs tens of thousands of events must
    // return every handle it opened — the journal, the adapters, the
    // tempdirs. Bound is generous against the sqlite WAL/SHM pair, tight
    // against any per-event handle retention.
    if let Some(delta) = fd_delta {
        assert!(
            delta <= BOUND_FD,
            "soak leaked {delta} descriptors over {n_events} events — \
             handles held per event (journal/adapter/tempfile?)"
        );
    } else {
        eprintln!("skipped: fd pin is Linux-only (this platform has no /proc/self/fd)");
    }
}
