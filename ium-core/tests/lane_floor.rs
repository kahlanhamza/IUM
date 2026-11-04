//! Lane-aware chunk floor (issue #44) — the 5 MiB `CHUNK_MIN` exists because
//! S3 multipart demands ≥ 5 MiB per non-final part. TUS has NO protocol
//! minimum, yet the global floor was applied there too: at the owner's
//! 3 KB/s bad moment a 5 MiB piece confirms every ~29 min, and the resplit
//! could never help (it floors at CHUNK_MIN). ADR-011 (adapter declares,
//! planner obeys) extends to the FLOOR: tus declares a 64 KiB lane floor —
//! a piece confirms in ~30 s at the bad-moment rate — and the planner's
//! floor regime runs against the LANE floor, not the S3 constant.

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::tus_mock::TusMock;
use ium_core::adapter::TransferAdapter;
use ium_core::adapter_tus::TusAdapter;
use ium_core::eviction::{EvictionPolicy, ResplitPolicy};
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

const KIB: u64 = 1024;

/// 1. The declaration: tus's ProbeReport carries the lane floor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tus_probe_declares_the_lane_floor() {
    let mock = TusMock::spawn(vec![], 0);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("src.bin");
    std::fs::write(&path, b"x").expect("write");

    let adapter = TusAdapter::new();
    let report = adapter
        .probe(&JobSpec {
            source_path: path.to_string_lossy().into_owned(),
            source_size: 1,
            adapter: "tus".into(),
            endpoint: mock.endpoint(),
        })
        .await
        .expect("creation");

    assert_eq!(
        report.min_chunk_bytes,
        Some(64 * KIB),
        "tus must DECLARE its lane floor through the ADR-011 channel — \
         64 KiB confirms in ~30 s at the owner's 3 KB/s bad moment"
    );
}

/// 2. The obedience: on a slow tus lane the resplit must be able to shape
///    pieces BELOW the 5 MiB S3 minimum. 1 MiB over two 512 KiB pieces, the
///    whole lane paced at 64 KiB/s: the fleet median is 64 KiB/s, so the
///    resplit targets ~32 KiB of wire — clamped UP to the LANE floor
///    (64 KiB), not to CHUNK_MIN (5 MiB). The pending 512 KiB piece must
///    split into 64 KiB pieces and the job must stay byte-exact.
///
///    RED FOR #47 (ignored until it lands): the drill exposed a manager-level
///    race BEYOND #44's scope — after the resplit retires the in-flight
///    1 MiB lease, piece 0's journal row ends LEASED with no owner and no
///    journaled resolution (no busy/confirm/error/eviction event), the drive
///    round sees pending==0, exits to Finishing, and the verify gate fails
///    loudly (verify_inconsistent). #44's core — the declaration, the
///    lane-floor target (65536 in the chunk_resplit payload), 16 sub-5MiB
///    pieces, the confirm-full absorption of resplit drift, and the 423
///    Busy-wait — is proven by the flow that DID complete (pieces 1-15,
///    byte-exact store) and by tus_probe_declares_the_lane_floor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_tus_lane_resplits_below_the_s3_minimum() {
    let span: u64 = 1024 * KIB;
    let mock = TusMock::spawn(vec![], span);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("src.bin");
    let source: Vec<u8> = (0..span as usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &source).expect("write source");

    // THE UPLINK: every PATCH body read is paced to 64 KiB/s.
    mock.set_read_pace_bps(Some(64.0 * KIB as f64));

    let db = dir.path().join("journal.db");
    let journal = SessionJournal::open(&db).expect("journal");
    let mut m = JobManager::with_journal(journal).expect("manager");
    m.set_resplit_policy(ResplitPolicy {
        target_secs: 0.5,
        slow_confirm: Duration::from_secs(4),
        min_change_ratio: 0.1,
    });
    m.set_eviction_policy(EvictionPolicy {
        window: Duration::from_millis(500),
        floor_ratio: 0.4,
        strikes_to_evict: 4,
    });
    m.register_adapter(Arc::new(TusAdapter::new()));

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: span,
        adapter: "tus".into(),
        endpoint: mock.endpoint(),
    };
    let rec = m.create_job(&spec, 2).expect("job"); // ONE 1 MiB piece: the initial plan runs at the global 5 MiB floor — the probe has not declared the lane yet; the resplit below is what re-shapes it
    let done = tokio::time::timeout(Duration::from_secs(120), m.run_job(&rec.id))
        .await
        .expect("paced job must finish inside the drill budget")
        .expect("run");
    if done.state != JobState::Done {
        let j = SessionJournal::open(&db).expect("journal reopen");
        for (_, kind, payload) in j
            .events_for(&rec.id)
            .expect("events")
            .into_iter()
            .rev()
            .take(80)
        {
            eprintln!("[drill] {kind}: {payload}");
        }
    }
    assert_eq!(done.state, JobState::Done);

    // Journal truth: the resplit happened at the LANE floor.
    let j = SessionJournal::open(&db).expect("journal reopen");
    let events = j.events_for(&rec.id).expect("events");
    let resplits: Vec<&(i64, String, String)> = events
        .iter()
        .filter(|(_, k, _)| k == "chunk_resplit")
        .collect();
    assert!(
        !resplits.is_empty(),
        "the slow lane must have triggered a resplit"
    );
    let payload: serde_json::Value =
        serde_json::from_str(&resplits[0].2).expect("resplit payload json");
    assert_eq!(
        payload["target"].as_u64(),
        Some(64 * KIB),
        "the resplit target must clamp to the LANE floor (64 KiB), never to \
         the 5 MiB S3 constant — payload: {payload}"
    );

    // The plan obeys: sub-5MiB pieces exist (the pending 512 KiB piece
    // split into lane-floor pieces).
    let chunks = j.load_chunks(&rec.id).expect("chunks");
    let lane_pieces = chunks
        .iter()
        .filter(|c| c.end - c.begin == 64 * KIB)
        .count();
    assert!(
        lane_pieces >= 8,
        "the pending piece must split into 64 KiB lane-floor pieces; got {lane_pieces} \
         across {} chunks",
        chunks.len()
    );
    assert!(
        chunks.iter().all(|c| c.end - c.begin <= 512 * KIB),
        "no piece may exceed the original plan"
    );

    // Byte-exact through the whole dance.
    assert_eq!(
        mock.stored(),
        source,
        "byte-exact through the lane-floor resplit"
    );
}
