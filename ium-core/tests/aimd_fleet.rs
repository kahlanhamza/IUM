//! AIMD fleet control (issue #49, M8 rung 2) + the measured-seed channel
//! (issue #48, M8 rung 1): the driver's worker waterline is SEEDED from the
//! journaled link profile (or the last adjustment) and then ADAPTED live:
//! a driver-side ticker folds each second's completed-call evidence into a
//! decision — additive increase while goodput improves, multiplicative
//! decrease when errors say the bearer is drowning, hard stop at the
//! plateau, and feedback gates so windows that moved too little to judge
//! hold instead of guessing. Workers above the waterline PARK at their
//! lease boundary and wake on growth — both directions bind MID-round.
//!
//! The owner's link swings 54x within an hour; the engine must follow it
//! in BOTH directions.
//!
//! What each layer proves:
//! 1. the pure decision module (`ium_core::fleet`) — every arm unit-tested;
//! 2. ENFORCEMENT of a waterline below the ceiling: a journaled
//!    `link_profile`/`fleet_adjusted` seed caps the first wave
//!    deterministically (workers park BEFORE their first call);
//! 3. the DECISION machinery: a scripted error storm forces a journaled
//!    `fleet_adjusted` decrease while the job is still running.

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::tus_mock::TusMock;
use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

// -- the pure decision module (issue #49) -----------------------------------

use ium_core::fleet::{fleet_for, AdaptiveFleet, FleetReason};

const MIB: u64 = 1024 * 1024;

#[test]
fn fleet_formula_composes_every_bound() {
    // pending and hard bounds compose; nothing exceeds any of them.
    assert_eq!(fleet_for(100, 32, None, None), 32);
    assert_eq!(fleet_for(3, 32, None, None), 3, "pending caps");
    assert_eq!(
        fleet_for(100, 32, Some(4), None),
        4,
        "adapter declaration caps"
    );
    assert_eq!(
        fleet_for(100, 32, Some(0), None),
        1,
        "a declared 0 clamps to 1"
    );
    assert_eq!(fleet_for(100, 32, None, Some(2)), 2, "measured seed caps");
    assert_eq!(
        fleet_for(100, 32, Some(8), Some(2)),
        2,
        "the tightest bound wins"
    );
    assert_eq!(fleet_for(100, 32, Some(2), Some(8)), 2);
    assert_eq!(
        fleet_for(0, 32, None, None),
        0,
        "nothing pending — no workers"
    );
}

#[test]
fn aimd_errors_decrease_multiplicatively() {
    let mut f = AdaptiveFleet::seeded(8);
    // An error storm: 8 completed calls, all errors. ZERO confirmed bytes —
    // the error verdict must not need goodput to speak.
    let change = f
        .step(0, 1.0, 8, 8, 32)
        .expect("a full error storm must decide");
    assert_eq!(change.from, 8);
    assert_eq!(change.to, 5, "floor(8 x 0.7)");
    assert_eq!(change.reason, FleetReason::Errors);
    assert_eq!(f.current(), 5);
    // A fleet already at 1 stays at 1 — no event for a non-change.
    let mut one = AdaptiveFleet::seeded(1);
    one.step(MIB, 1.0, 4, 0, 32); // baseline
    assert!(
        one.step(MIB, 1.0, 4, 4, 32).is_none(),
        "1 x 0.7 floors at 1: no change, no event"
    );
    // Too few completed calls: one unlucky call is not a drowning bearer.
    let mut quiet = AdaptiveFleet::seeded(8);
    assert!(
        quiet.step(0, 1.0, 2, 2, 32).is_none(),
        "sub-4-call windows carry no error verdict"
    );
}

#[test]
fn aimd_grows_additively_and_stops_at_the_ceiling() {
    let mut f = AdaptiveFleet::seeded(1);
    // First clean window sets the baseline — it does not grow on itself.
    assert!(
        f.step(MIB, 1.0, 4, 0, 32).is_none(),
        "the first measured window is the baseline, not a growth signal"
    );
    // A meaningfully better window (+8%) grows by the additive step.
    let change = f
        .step(2 * MIB, 1.0, 4, 0, 32)
        .expect("2x the baseline must grow");
    assert_eq!((change.from, change.to), (1, 3));
    assert_eq!(change.reason, FleetReason::Growth);
    // ...up to the ceiling, where growth stops silently (plateau at the top).
    let mut near = AdaptiveFleet::seeded(31);
    near.step(MIB, 1.0, 4, 0, 32); // baseline
    let change = near.step(2 * MIB, 1.0, 4, 0, 32).expect("growth attempt");
    assert_eq!(change.to, 32, "ceiling clamp");
    assert!(
        near.step(2 * MIB, 1.0, 4, 0, 32).is_none(),
        "at the ceiling there is nothing to add"
    );
}

#[test]
fn aimd_overshoot_shrinks_by_one_and_the_plateau_holds() {
    let mut f = AdaptiveFleet::seeded(4);
    f.step(MIB, 1.0, 4, 0, 32); // baseline 1 MiB/s
                                // A meaningfully WORSE window with no errors: we overshot the bearer.
    let change = f
        .step(MIB / 2, 1.0, 4, 0, 32)
        .expect("a 50% regression must shrink");
    assert_eq!((change.from, change.to), (4, 3));
    assert_eq!(change.reason, FleetReason::Overshoot);
    // The plateau: between 0.9x and 1.08x of the best, hold.
    let mut g = AdaptiveFleet::seeded(4);
    g.step(MIB, 1.0, 4, 0, 32); // baseline
    assert!(
        g.step(MIB, 1.0, 4, 0, 32).is_none(),
        "a flat window holds the fleet"
    );
}

#[test]
fn aimd_feedback_gates_hold_unjudgeable_windows() {
    let mut f = AdaptiveFleet::seeded(4);
    // Too few bytes to judge goodput (the 3 KB/s atomic-S3 regime —
    // resplit's domain, not AIMD's).
    assert!(
        f.step(1024, 10.0, 8, 0, 32).is_none(),
        "sub-64-KiB windows carry no goodput verdict"
    );
    // Too short to judge (sub-second samples).
    assert!(
        f.step(4 * MIB, 0.1, 8, 0, 32).is_none(),
        "sub-0.5 s windows carry no goodput verdict"
    );
    assert_eq!(f.current(), 4, "held windows never move the fleet");
}

// -- the manager-level proof -------------------------------------------------

/// A counting double: no protocol, pure fleet semantics. `errors_left`
/// scripted errors fire as Transient (counted retries — the AIMD error
/// signal); every clean call confirms its whole lease. Concurrency is
/// recorded per EPOCH, where an epoch ends when concurrency returns to
/// zero — a clean between-calls boundary.
struct AimdAdapter {
    errors_left: AtomicU32,
    calls: AtomicUsize,
    current: AtomicUsize,
    epoch_max: AtomicUsize,
    epochs: std::sync::Mutex<Vec<usize>>,
    /// Simulated wire time per call. Long enough that the FIRST evidence
    /// window (errors only) closes well before the pool drains, so the
    /// decrease is journaled while the job is still running.
    pace: Duration,
}

impl AimdAdapter {
    fn new(errors: u32, pace: Duration) -> Arc<Self> {
        Arc::new(Self {
            errors_left: AtomicU32::new(errors),
            calls: AtomicUsize::new(0),
            current: AtomicUsize::new(0),
            epoch_max: AtomicUsize::new(0),
            epochs: std::sync::Mutex::new(Vec::new()),
            pace,
        })
    }

    fn epochs(&self) -> Vec<usize> {
        self.epochs.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl TransferAdapter for AimdAdapter {
    fn name(&self) -> &'static str {
        "aimd_mock"
    }
    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            max_parallel: None,
            min_chunk_bytes: None,
            session_token: Some("aimd-session".into()),
            concatenation: false,
        })
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        lease: &Lease,
        _progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let _call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.epoch_max.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.pace).await;
        // The scripted error, then the clean confirm — the drop to zero at
        // the end closes the epoch. CAS, not fetch_sub: a fetch_sub on 0
        // would WRAP to u32::MAX and every later call would "error" forever
        // (the underflow this harness's first draft died of — the test must
        // be more honest than the code it tests).
        let should_error = loop {
            let cur = self.errors_left.load(Ordering::SeqCst);
            if cur == 0 {
                break false;
            }
            if self
                .errors_left
                .compare_exchange(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break true;
            }
        };
        let result = if should_error {
            Err(AdapterError::Transient("scripted aimd error".into()))
        } else {
            Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.resume_from,
                adapter_token: None,
            })
        };
        let prev = self.current.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            let max = self.epoch_max.swap(0, Ordering::SeqCst);
            self.epochs.lock().unwrap().push(max);
        }
        result
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("aimd double: journal-side gate only".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

const SPAN: u64 = 40 * MIB; // 8 pieces of exactly 5 MiB (the CHUNK_MIN floor)

fn spec_for(dir: &tempfile::TempDir) -> JobSpec {
    let path = dir.path().join("src.bin");
    let f = std::fs::File::create(&path).expect("create source");
    f.set_len(SPAN)
        .expect("sparse source — the double never reads it");
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "aimd_mock".into(),
        endpoint: "unused://aimd".into(),
    }
}

fn events(db: &std::path::Path, job: &str) -> Vec<(i64, String, String)> {
    SessionJournal::open(db)
        .expect("journal reopen")
        .events_for(job)
        .expect("events")
}

fn fleet_adjusted(events: &[(i64, String, String)]) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter(|(_, k, _)| k == "fleet_adjusted")
        .map(|(_, _, p)| serde_json::from_str(p).expect("fleet_adjusted json"))
        .collect()
}

/// The DECISION machinery: a full error storm in the first evidence window
/// forces a journaled multiplicative decrease WHILE the job runs — the
/// journal records {from: 8, to: 5, reason: "errors"} and the job still
/// completes. (The seeded-waterline ENFORCEMENT is proven by the next two
/// tests, which are race-free by construction.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scripted_errors_decrease_the_fleet_and_journal_the_reason() {
    // 2 s of simulated wire: the error wave completes at ~2 s, the ticker's
    // window folds it at ~3 s, and the pool (re-lease + confirm waves)
    // drains well after — the decision lands mid-job, deterministically.
    let adapter = AimdAdapter::new(8, Duration::from_secs(2));
    let dir = tempfile::tempdir().expect("tempdir");
    let spec = spec_for(&dir);
    let db = dir.path().join("journal.db");
    let mut m =
        JobManager::with_journal(SessionJournal::open(&db).expect("journal")).expect("manager");
    m.register_adapter(adapter.clone());
    let rec = m.create_job(&spec, 8).expect("job");
    let done = tokio::time::timeout(Duration::from_secs(90), m.run_job(&rec.id))
        .await
        .expect("job must finish")
        .expect("run");
    if done.state != JobState::Done {
        for (_, kind, payload) in events(&db, &rec.id).into_iter().rev().take(40) {
            eprintln!("[aimd] {kind}: {payload}");
        }
    }
    assert_eq!(done.state, JobState::Done);

    let evs = events(&db, &rec.id);
    let adjustments = fleet_adjusted(&evs);
    assert!(
        !adjustments.is_empty(),
        "the error storm must produce at least one journaled decision"
    );
    assert_eq!(
        adjustments[0]["from"], 8,
        "wave 1 ran at the seeded ceiling"
    );
    assert_eq!(
        adjustments[0]["to"], 5,
        "floor(8 x 0.7) after a 1.0 error rate"
    );
    assert_eq!(adjustments[0]["reason"], "errors");
    assert_eq!(
        adapter.epochs()[0],
        8,
        "the first wave provably ran 8 concurrent workers"
    );
}

/// ENFORCEMENT, race-free by construction: the journaled measured seed
/// (issue #48's channel) caps the waterline BELOW the ceiling from the
/// very first call — workers 2..7 park BEFORE leasing, so the first wave
/// provably runs exactly 2 concurrent workers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn journaled_link_profile_caps_the_first_round() {
    // 150 ms/call: the whole job (8 chunks / 2 workers) drains inside ONE
    // evidence window, so the ticker never fires — the seed alone binds,
    // race-free. (Adaptation FROM a seed is proven by the growth arm's pure
    // tests and by the errors test's journaled decrease.)
    let adapter = AimdAdapter::new(0, Duration::from_millis(150));
    let dir = tempfile::tempdir().expect("tempdir");
    let spec = spec_for(&dir);
    let db = dir.path().join("journal.db");
    let mut m =
        JobManager::with_journal(SessionJournal::open(&db).expect("journal")).expect("manager");
    m.register_adapter(adapter.clone());
    let rec = m.create_job(&spec, 8).expect("job");
    m.journal()
        .append_event(
            &rec.id,
            "link_profile",
            &serde_json::json!({
                "single_bps": 65536.0,
                "k_bps": 65536.0,
                "k": 4,
                "class": "bearer_cap",
                "recommended_piece_bytes": 5 * MIB,
                "recommended_fleet": 2,
            }),
        )
        .expect("seed the measured profile");
    let done = tokio::time::timeout(Duration::from_secs(60), m.run_job(&rec.id))
        .await
        .expect("job must finish")
        .expect("run");
    assert_eq!(done.state, JobState::Done);
    let epochs = adapter.epochs();
    assert_eq!(
        epochs.first().copied(),
        Some(2),
        "the measured recommendation caps the FIRST wave at 2"
    );
    assert_eq!(
        epochs.iter().max().copied(),
        Some(2),
        "no wave may ever exceed the measured cap; got {epochs:?}"
    );
    let adjustments = fleet_adjusted(&events(&db, &rec.id));
    assert!(
        adjustments.is_empty(),
        "a steady job at its measured size decides nothing — got {adjustments:?}"
    );
}

/// Resume re-derivation (ADR-002): the LAST `fleet_adjusted` event seeds a
/// fresh manager — no process memory survives a crash, the journal does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn last_fleet_adjusted_seeds_a_fresh_manager() {
    let adapter = AimdAdapter::new(0, Duration::from_millis(150));
    let dir = tempfile::tempdir().expect("tempdir");
    let spec = spec_for(&dir);
    let db = dir.path().join("journal.db");
    let mut m =
        JobManager::with_journal(SessionJournal::open(&db).expect("journal")).expect("manager");
    m.register_adapter(adapter.clone());
    let rec = m.create_job(&spec, 8).expect("job");
    m.journal()
        .append_event(
            &rec.id,
            "fleet_adjusted",
            &serde_json::json!({"from": 8, "to": 3, "reason": "errors", "goodput_bps": 1000.0}),
        )
        .expect("seed the prior decision");
    let done = tokio::time::timeout(Duration::from_secs(60), m.run_job(&rec.id))
        .await
        .expect("job must finish")
        .expect("run");
    assert_eq!(done.state, JobState::Done);
    let epochs = adapter.epochs();
    assert_eq!(
        epochs.first().copied(),
        Some(3),
        "the journaled decision caps the FIRST wave after the restart"
    );
    assert_eq!(
        epochs.iter().max().copied(),
        Some(3),
        "no wave may ever exceed the journaled cap (the tail may run \
         narrower — fewer pending chunks); got {epochs:?}"
    );
}

/// Default-off: without `IUM_LINK_PROBE`, a managed tus job journals NO
/// link-profile events — the prober is a deliberate instrument on a metered
/// bearer, not ambient traffic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_link_probe_without_the_opt_in() {
    let span: u64 = 256 * 1024;
    let mock = TusMock::spawn(vec![], span);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("src.bin");
    let source: Vec<u8> = (0..span as usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &source).expect("write source");
    let db = dir.path().join("journal.db");
    let mut m =
        JobManager::with_journal(SessionJournal::open(&db).expect("journal")).expect("manager");
    m.register_adapter(Arc::new(ium_core::adapter_tus::TusAdapter::new()));
    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: span,
        adapter: "tus".into(),
        endpoint: mock.endpoint(),
    };
    let rec = m.create_job(&spec, 2).expect("job");
    let done = tokio::time::timeout(Duration::from_secs(60), m.run_job(&rec.id))
        .await
        .expect("finish")
        .expect("run");
    assert_eq!(done.state, JobState::Done);
    let evs = events(&db, &rec.id);
    assert!(
        !evs.iter()
            .any(|(_, k, _)| k == "link_profile" || k == "link_profile_skipped"),
        "no probe events without the opt-in"
    );
}
