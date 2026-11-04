//! Slow-lane drills for the tus client (issue #42) — the request layer must
//! stop owning DURATION and start owning PROGRESS.
//!
//! The wall, from the issue: `TusAdapter::new` builds reqwest with a 600 s
//! TOTAL per-request timeout, body streaming included. A 5 MiB piece needs
//! ≥ 600 s whenever the uplink is below ~8.7 KB/s — below that line every
//! first PATCH dies at the wall, the realign dies the same way, and the
//! second conflict is terminal. The owner's 3 KB/s tunnel moment sits
//! exactly there: the tus lane was structurally non-convergent.
//!
//! The fix's contract, pinned here at compressed timescales:
//! 1. a steady slow flow (bytes moving) is NEVER cut, no matter how long —
//!    `with_idle(1s)` riding a 4-second trickle crosses four idle windows;
//! 2. a stalled flow (no bytes at all) is cut at ~idle and classified
//!    `Transient` (a counted retry), never `Unknown`;
//! 3. after the stall clears, the same lease converges from server truth.

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::time::{Duration, Instant};

use common::tus_mock::{PatchBehavior, TusMock};
use ium_core::adapter::{AdapterError, ProgressReporter, TransferAdapter};
use ium_core::adapter_tus::TusAdapter;
use ium_core::{JobSpec, Lease};

const KIB: u64 = 1024;

fn spec_for(mock: &TusMock, path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "tus".into(),
        endpoint: mock.endpoint(),
    }
}

fn lease_of(span: u64) -> Lease {
    Lease {
        idx: 0,
        begin: 0,
        end: span,
        resume_from: 0,
        attempt: 0,
    }
}

fn write_source(dir: &tempfile::TempDir, size: u64) -> std::path::PathBuf {
    let path = dir.path().join("source.bin");
    std::fs::write(
        &path,
        (0..size as usize)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>(),
    )
    .expect("write source");
    path
}

/// 1. A steady slow flow is never cut. 2 MiB paced at 512 KiB/s ≈ 4 s of
///    continuous wire drain — four times the 1-second window any
///    duration-based regression would cut at. The wall is gone: the lane
///    rides for as long as it takes, and the only tripwire is idleness
///    (4 s of drain ≪ the 10 s window here).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn steady_trickle_crosses_idle_windows_and_completes() {
    let span = 2 * 1024 * KIB;
    let mock = TusMock::spawn(
        vec![PatchBehavior::Pace {
            bps: 512.0 * 1024.0,
        }],
        span,
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(&dir, span);
    let spec = spec_for(&mock, &path, span);

    let adapter = TusAdapter::with_idle(Duration::from_secs(10));
    adapter.probe(&spec).await.expect("creation");

    let progress = ProgressReporter::new();
    let started = Instant::now();
    let confirm = tokio::time::timeout(
        Duration::from_secs(60),
        adapter.upload_chunk(&spec, &lease_of(span), &progress),
    )
    .await
    .expect("trickle drill must finish well inside the test budget")
    .expect("a steadily-progressing flow must never be aborted, whatever its duration");

    assert_eq!(confirm.bytes_confirmed, span);
    assert_eq!(mock.offset(), span);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(30),
        "a 4 s trickle took {elapsed:?} — a duration-based cut is back"
    );
    assert_eq!(mock.patches_completed(), 1, "one PATCH, no realign dance");
    assert_eq!(
        mock.stored(),
        std::fs::read(&path).expect("read source")[..span as usize],
        "byte-exact through the paced lane"
    );
}

/// 2. A stalled flow is cut at ~idle and classified `Transient` — a counted
///    retry, never `Unknown`. The server holds the connection open reading
///    nothing (dead tunnel, full buffers); the watchdog must end the PATCH
///    in roughly the idle window, not hold it for minutes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_body_is_cut_at_idle_and_classified_transient() {
    let span = 2 * 1024 * KIB;
    let mock = TusMock::spawn(vec![PatchBehavior::Stall], span);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(&dir, span);
    let spec = spec_for(&mock, &path, span);

    let adapter = TusAdapter::with_idle(Duration::from_secs(1));
    adapter.probe(&spec).await.expect("creation");

    let progress = ProgressReporter::new();
    let started = Instant::now();
    let err = tokio::time::timeout(
        Duration::from_secs(15),
        adapter.upload_chunk(&spec, &lease_of(span), &progress),
    )
    .await
    .expect("a 1 s idle cut must not need a 15 s drill budget")
    .expect_err("a fully stalled body must fail");

    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "idle cut took {elapsed:?} — must be ~idle, not a slow death"
    );
    assert!(
        matches!(err, AdapterError::Transient(_)),
        "an idle-progress cut is a counted retry (Transient), got: {err:?}"
    );
    assert_eq!(mock.offset(), 0, "the stalled server persisted nothing");
    assert_eq!(mock.patches_completed(), 0);

    // 3. The link recovers: the SAME lease converges from server truth.
    //    (Script exhausted → further PATCHes are Accept.)
    let progress2 = ProgressReporter::new();
    let confirm = tokio::time::timeout(
        Duration::from_secs(15),
        adapter.upload_chunk(&spec, &lease_of(span), &progress2),
    )
    .await
    .expect("recovery upload must finish inside the budget")
    .expect("after the stall clears the lease must converge from server truth");
    assert_eq!(confirm.bytes_confirmed, span);
    assert_eq!(mock.offset(), span);
}
