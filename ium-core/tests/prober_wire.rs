//! The link prober, WIRED (issue #48, M8 rung 1): a managed tus job with
//! `IUM_LINK_PROBE=1` must MEASURE its link at the probe boundary, journal
//! the profile once, and let the planner obey the measured piece size.
//! The canaries are PACED (deterministic per-flow-cap shape: each
//! connection gets its own 64 KiB/s), so the classification is stable —
//! the K-stream phase must measure ~4x the single-stream phase.
//!
//! Opt-in by env: every byte is sacred on a metered bearer; the prober is
//! a deliberate instrument (the default-off face is pinned in
//! `aimd_fleet.rs::no_link_probe_without_the_opt_in`).

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::tus_mock::{PatchBehavior, TusMock};
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const SPAN: u64 = 4 * MIB; // 2 planned pieces of 2 MiB — the measured piece must split them

fn payload(dir: &tempfile::TempDir) -> Vec<u8> {
    let path = dir.path().join("src.bin");
    let source: Vec<u8> = (0..SPAN as usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &source).expect("write source");
    source
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn managed_tus_job_probes_journals_and_replans_to_the_measured_piece() {
    std::env::set_var("IUM_LINK_PROBE", "1");

    // Five paced canaries: 1 single-stream + K=4 aggregate. Each connection
    // gets its OWN 64 KiB/s — the per-flow-cap shape (the IDM case).
    let pace = PatchBehavior::Pace {
        bps: 64.0 * KIB as f64,
    };
    let mock = TusMock::spawn(vec![pace.clone(); 5], SPAN);
    let dir = tempfile::tempdir().expect("tempdir");
    let source = payload(&dir);
    let path = dir.path().join("src.bin");

    let db = dir.path().join("journal.db");
    let mut m =
        JobManager::with_journal(SessionJournal::open(&db).expect("journal")).expect("manager");
    m.register_adapter(Arc::new(ium_core::adapter_tus::TusAdapter::new()));
    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "tus".into(),
        endpoint: mock.endpoint(),
    };
    let rec = m.create_job(&spec, 2).expect("job");
    let done = tokio::time::timeout(Duration::from_secs(120), m.run_job(&rec.id))
        .await
        .expect("paced probe + fast upload must finish inside the drill budget")
        .expect("run");
    if done.state != JobState::Done {
        let j = SessionJournal::open(&db).expect("journal reopen");
        for (_, kind, payload) in j
            .events_for(&rec.id)
            .expect("events")
            .into_iter()
            .rev()
            .take(60)
        {
            eprintln!("[drill] {kind}: {payload}");
        }
    }
    assert_eq!(
        done.state,
        JobState::Done,
        "the measured plan must still finish byte-exact"
    );

    let j = SessionJournal::open(&db).expect("journal reopen");
    let evs = j.events_for(&rec.id).expect("events");

    // The measurement, journaled EXACTLY once at the probe boundary.
    let profiles: Vec<&(i64, String, String)> =
        evs.iter().filter(|(_, k, _)| k == "link_profile").collect();
    assert_eq!(profiles.len(), 1, "one probing pass per job");
    let profile: serde_json::Value =
        serde_json::from_str(&profiles[0].2).expect("link_profile json");
    assert_eq!(
        profile["class"], "per_flow_cap",
        "K paced connections must measure ~4x the single stream — profile: {profile}"
    );
    assert_eq!(profile["recommended_fleet"], 4, "fleet = the measured K");

    // The planner OBEYS: the measured piece (~20 s of the measured single
    // stream = ~1.25 MiB) binds below the planned 2 MiB pieces — the pool
    // re-plans while everything is still pending.
    let replans: Vec<&(i64, String, String)> = evs
        .iter()
        .filter(|(_, k, _)| k == "replan_measured")
        .collect();
    assert_eq!(replans.len(), 1, "one measured replan");
    let replan: serde_json::Value = serde_json::from_str(&replans[0].2).expect("replan json");
    let recommended = profile["recommended_piece_bytes"].as_u64().expect("piece");
    assert_eq!(
        replan["applied_piece"], recommended,
        "no adapter cap on this lane: the measured piece is applied as measured"
    );
    let chunks = j.load_chunks(&rec.id).expect("chunks");
    assert!(
        chunks.len() >= 4,
        "4 MiB at ~1.25 MiB pieces must re-plan into >= 4 pieces; got {}",
        chunks.len()
    );
    assert!(
        chunks.iter().all(|c| c.end - c.begin <= recommended),
        "no piece may exceed the measured recommendation"
    );

    // Byte-exact through the whole dance — canaries ride their own sessions.
    assert_eq!(
        mock.stored(),
        source,
        "byte-exact through the measured plan"
    );
}
