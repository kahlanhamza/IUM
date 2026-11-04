//! The link prober drills (issue #41): the 1-vs-K question answered by
//! MEASUREMENT, both ways. Two server shapes, the two possible answers:
//!
//! * per-flow caps (each connection gets its own rate — the classic
//!   per-connection throttle): K streams multiply the aggregate;
//! * a shared bearer (the owner's tunnel): K streams change nothing.
//!
//! The prober must classify each correctly and recommend the fleet the
//! measurement supports. These run on the PR lane (compressed canaries,
//! seconds each) — they ARE the unit layer of the #45 ladder.

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::time::Duration;

use common::tus_mock::TusMock;
use ium_core::prober::{probe_link, LinkClass};

const KIB: u64 = 1024;
const SPAN: u64 = 256 * KIB;

fn payload(dir: &tempfile::TempDir) -> (std::path::PathBuf, Vec<u8>) {
    let path = dir.path().join("payload.bin");
    let source: Vec<u8> = (0..SPAN as usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &source).expect("write payload");
    (path, source)
}

/// Per-flow caps: each connection is throttled to 8 KiB/s; K=4 streams
/// must aggregate ~4×. The prober must classify PerFlowCap and recommend
/// the fleet it measured.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn per_flow_caps_classify_and_recommend_fleet() {
    let mock = TusMock::spawn(vec![], SPAN);
    mock.set_read_pace_bps(Some(8.0 * 1024.0)); // per-CONNECTION pace

    let dir = tempfile::tempdir().expect("tempdir");
    let (path, _source) = payload(&dir);

    let profile = tokio::time::timeout(
        Duration::from_secs(120),
        probe_link(&path, SPAN, &mock.endpoint(), 4, Some(8.0 * 1024.0)),
    )
    .await
    .expect("probing must finish inside the drill budget")
    .expect("probe");

    eprintln!(
        "[prober] per-flow: s1={:.0} B/s k={:.0} B/s class={:?} fleet={} piece={}",
        profile.single_bps,
        profile.k_bps,
        profile.class,
        profile.recommended_fleet,
        profile.recommended_piece_bytes
    );
    assert_eq!(
        profile.class,
        LinkClass::PerFlowCap,
        "4 streams at ~4x aggregate must classify per-flow-capped (s1={:.0}, sk={:.0})",
        profile.single_bps,
        profile.k_bps
    );
    assert_eq!(profile.recommended_fleet, 4, "fleet = the measured K");
    // Piece = ~20 s of the MEASURED single-stream wire, clamped to
    // [lane floor, engine max] — with S1 ≈ 8 KiB/s that is ~160 KiB,
    // comfortably inside the clamp; assert the BOUNDS, not the exact
    // measured number.
    assert!(
        (64 * KIB..=ium_core::chunk::CHUNK_MAX).contains(&profile.recommended_piece_bytes),
        "piece {} out of the lane-floor..engine-max clamp",
        profile.recommended_piece_bytes
    );
}

/// A shared bearer: the aggregate NEVER exceeds the bearer, whatever K is.
/// The prober must classify BearerCap and recommend a fleet of 1 — adding
/// streams would only contend.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn shared_bearer_classifies_and_stays_single() {
    let mock = TusMock::spawn(vec![], SPAN);
    mock.set_bearer_pace_bps(Some(8.0 * 1024.0)); // TOTAL across connections

    let dir = tempfile::tempdir().expect("tempdir");
    let (path, _source) = payload(&dir);

    let profile = tokio::time::timeout(
        Duration::from_secs(120),
        probe_link(&path, SPAN, &mock.endpoint(), 4, Some(8.0 * 1024.0)),
    )
    .await
    .expect("probing must finish inside the drill budget")
    .expect("probe");

    eprintln!(
        "[prober] bearer: s1={:.0} B/s k={:.0} B/s class={:?} fleet={}",
        profile.single_bps, profile.k_bps, profile.class, profile.recommended_fleet
    );
    assert_eq!(
        profile.class,
        LinkClass::BearerCap,
        "K streams on one bearer must NOT multiply (s1={:.0}, sk={:.0})",
        profile.single_bps,
        profile.k_bps
    );
    assert_eq!(
        profile.recommended_fleet, 1,
        "one bearer → one stream; more would only contend"
    );
}
