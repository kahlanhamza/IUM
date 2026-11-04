//! The throttled-link ladder (issue #45) — the KB/s tail pinned forever.
//!
//! The wall was discovered by the OWNER IN THE FIELD (a 3 KB/s tunnel
//! moment), not by CI — the exact gap drills exist to close. This ladder
//! drives REAL engine runs (create_job → run_job → honest Done from the
//! journal) through the tus-shaped loopback at the owner's rate ladder:
//!
//! * **3 KB/s** — the bad-moment rate; the wall's edge (the #42 lane)
//! * **8 KB/s** — just above the old wall's line (~8.7 KB/s)
//! * **50 KB/s** — a bad mobile uplink
//! * **160 KB/s** — the owner's Google-speedtest moment (1.29 Mbps)
//!
//! Every leg asserts: convergent `Done`, byte-exact readback, and the
//! #44 overhead-honesty measurement (wire bytes vs payload, recorded, not
//! asserted blindly). The 3 KB/s leg's 64 KiB pieces are the lane-floor
//! leg: without #44 the initial 5 MiB plan would confirm every ~29 min
//! and this drill would be impossible at CI timescales.
//!
//! Env-gated: `IUM_THROTTLE_LADDER=1` (stress.yml; weekly + dispatch).
//! The PR lane skips LOUDLY — the compressed-timescale equivalents run
//! there in tus_slow_lane.rs.

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::tus_mock::TusMock;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal, TusAdapter};

const KIB: u64 = 1024;

/// One ladder leg: (rate B/s, payload bytes, what it pins).
const LEGS: &[(f64, u64, &str)] = &[
    (
        3.0 * 1024.0,
        64 * KIB,
        "3 KB/s — the bad-moment wall's edge (#42, #44 lane floor)",
    ),
    (
        8.0 * 1024.0,
        128 * KIB,
        "8 KB/s — just above the old 8.7 KB/s wall line",
    ),
    (50.0 * 1024.0, 512 * KIB, "50 KB/s — a bad mobile uplink"),
    (
        160.0 * 1024.0,
        1024 * KIB,
        "160 KB/s — the owner's 1.29 Mbps speedtest moment",
    ),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_kbps_ladder_converges_and_stays_byte_exact() {
    if std::env::var("IUM_THROTTLE_LADDER").as_deref() != Ok("1") {
        eprintln!(
            "skipped (loud, #45): set IUM_THROTTLE_LADDER=1 to run the KB/s \
             ladder — stress.yml owns it; the PR lane runs the compressed \
             tus_slow_lane drills instead"
        );
        return;
    }

    for &(rate, span, label) in LEGS {
        let mock = TusMock::spawn(vec![], span);
        // THE UPLINK: every body byte read at the leg's rate.
        mock.set_read_pace_bps(Some(rate));

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("payload.bin");
        let source: Vec<u8> = (0..span as usize).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &source).expect("write payload");

        let db = dir.path().join("journal.db");
        let journal = SessionJournal::open(&db).expect("journal");
        let mut m = JobManager::with_journal(journal).expect("manager");
        m.register_adapter(Arc::new(TusAdapter::new()));

        let spec = JobSpec {
            source_path: path.to_string_lossy().into_owned(),
            source_size: span,
            adapter: "tus".into(),
            endpoint: mock.endpoint(),
        };
        let rec = m.create_job(&spec, 1).expect("job");
        let started = Instant::now();
        let done = tokio::time::timeout(Duration::from_secs(180), m.run_job(&rec.id))
            .await
            .unwrap_or_else(|_| panic!("{label}: the leg outlived its 180 s budget"))
            .unwrap_or_else(|e| panic!("{label}: run error: {e}"));
        let elapsed = started.elapsed();

        assert_eq!(done.state, JobState::Done, "{label}: must converge Done");
        assert_eq!(
            mock.stored(),
            source,
            "{label}: byte-exact through the paced lane"
        );

        // The #44 overhead honesty, MEASURED: everything the server read
        // (request heads + bodies, upload direction) vs the payload. At
        // 64 KiB pieces the tus per-PATCH head overhead must stay tiny —
        // this is the number that says the lane floor is not a scam.
        let wire = mock.wire_bytes();
        let overhead_pct = 100.0 * (wire.saturating_sub(span)) as f64 / span as f64;
        eprintln!(
            "[ladder] {label}: {span} bytes in {elapsed:?} ({:.1} B/s achieved), \
             wire overhead {:.2}%",
            span as f64 / elapsed.as_secs_f64(),
            overhead_pct
        );
        assert!(
            overhead_pct < 5.0,
            "{label}: wire overhead {overhead_pct:.2}% — the protocol or the \
             planning is leaking bytes (heads/pieces too heavy for the floor)"
        );

        // Sanity bound, generous for loaded runners: the ladder legs ride
        // the pace, so elapsed ≈ span/rate + engine overhead — anything
        // beyond 3× the ideal wire time means something spun.
        let ideal = span as f64 / rate;
        assert!(
            elapsed.as_secs_f64() < 3.0 * ideal + 30.0,
            "{label}: took {elapsed:?} vs the {ideal:.0} s the wire alone needs — \
             the engine is adding duration, not removing it"
        );

        let _ = SessionJournal::open(&db).expect("journal reopen");
        drop(mock);
    }
}
