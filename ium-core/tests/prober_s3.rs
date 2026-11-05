//! Issue #57 (M9#2): the link prober covers the S3 lane. The RED artifact:
//! `classify` (the shared pure classification) does not exist yet and
//! `probe_link` is not yet a trait method — this file must FAIL to compile
//! on the clean tree, then go GREEN with the implementation.
//!
//! Three layers, per the issue's RED plan:
//! 1. the pure-fn classification contracts (the tus arithmetic, shared);
//! 2. the manager contracts for an S3-shaped lane (journal-once, obey,
//!    skip-on-error, silence for prober-less lanes);
//! 3. the lane-floor interaction — the S3 5 MiB multipart minimum MUST
//!    hold under an aggressive measured piece (the pool floors it).

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::prober::{LinkClass, LinkProfile};
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const SPAN: u64 = 12 * MIB; // the floor shape: 12 MiB floors a 64 KiB piece to 2 x 6 MiB

/// The opt-in is PROCESS-WIDE state and the tests in this file run in
/// parallel threads — every test holds this lock across its whole run so
/// one test's env flip can never race another's probe boundary. (tokio's
/// Mutex: the guard is deliberately held across awaits.)
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------------------------------------------------------------------------
// 1. the pure-fn classification contracts (shared by the tus and S3 legs)
// ---------------------------------------------------------------------------

#[test]
fn boundaries_are_inclusive_at_2x_and_exclusive_at_1_5x() {
    let big = 1 << 30;
    // 4x: per-flow caps — the IDM shape.
    let p = ium_core::prober::classify(100.0, 400.0, 4, big);
    assert_eq!(p.class, LinkClass::PerFlowCap);
    assert_eq!(p.recommended_fleet, 4);
    // exactly 2.0x: the >= is inclusive — PerFlowCap.
    let p = ium_core::prober::classify(100.0, 200.0, 4, big);
    assert_eq!(p.class, LinkClass::PerFlowCap);
    assert_eq!(p.recommended_fleet, 4);
    // 1.9x: between the lines — raise cautiously.
    let p = ium_core::prober::classify(100.0, 190.0, 4, big);
    assert_eq!(p.class, LinkClass::Mixed);
    assert_eq!(p.recommended_fleet, 2);
    // exactly 1.5x: NOT a bearer (the < is exclusive) — Mixed.
    let p = ium_core::prober::classify(100.0, 150.0, 4, big);
    assert_eq!(p.class, LinkClass::Mixed);
    // 1.49x: one bearer.
    let p = ium_core::prober::classify(100.0, 149.0, 4, big);
    assert_eq!(p.class, LinkClass::BearerCap);
    assert_eq!(p.recommended_fleet, 1);
    // flat: the owner's tunnel shape.
    let p = ium_core::prober::classify(100.0, 100.0, 4, big);
    assert_eq!(p.class, LinkClass::BearerCap);
}

#[test]
fn piece_is_20s_of_measured_wire_clamped_to_the_lane_window() {
    let big = 1 << 30;
    // 20 s of 100 B/s = 2 kB -> clamped UP to the 64 KiB canary floor.
    let p = ium_core::prober::classify(100.0, 400.0, 4, big);
    assert_eq!(p.recommended_piece_bytes, 64 * KIB);
    // 20 s of 1 MiB/s = 20 MiB -> clamped DOWN to the engine floor CHUNK_MIN.
    let p = ium_core::prober::classify(MIB as f64, 4.0 * MIB as f64, 4, big);
    assert_eq!(p.recommended_piece_bytes, 5 * MIB);
}

#[test]
fn piece_never_exceeds_the_source() {
    // a 1 KiB source can never recommend a 64 KiB piece
    let p = ium_core::prober::classify(100.0, 400.0, 4, 1024);
    assert_eq!(p.recommended_piece_bytes, 1024);
}

#[test]
fn journal_payload_carries_the_measured_piece_and_fleet() {
    let p = ium_core::prober::classify(100.0, 400.0, 4, 1 << 30);
    let v = ium_core::prober::journal_payload(&p);
    assert_eq!(v["class"], "per_flow_cap");
    assert_eq!(v["recommended_fleet"], 4);
    assert_eq!(v["k"], 4);
}

// ---------------------------------------------------------------------------
// 2. the manager contracts for an S3-shaped lane
// ---------------------------------------------------------------------------

/// What the scripted adapter's `probe_link` should answer.
enum ProbeScript {
    Profile(LinkProfile),
    Silent,             // Ok(None): the lane has no shaped prober
    Down(&'static str), // Err: the probe leg failed
}

/// A scripted S3-shaped lane: name "s3_multipart" (the registry key the
/// manager resolves), resumable, whole-span confirms, and a probe_link
/// the test controls. Every byte-confirmation is real file I/O.
struct ScriptedS3 {
    script: Mutex<Option<ProbeScript>>,
    probe_link_calls: Mutex<u32>,
}

impl ScriptedS3 {
    fn new(script: ProbeScript) -> Self {
        Self {
            script: Mutex::new(Some(script)),
            probe_link_calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl TransferAdapter for ScriptedS3 {
    fn name(&self) -> &'static str {
        "s3_multipart"
    }
    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE | Capability::DYNAMIC_CHUNKS)
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
            session_token: Some("scripted-s3-upload".into()),
            concatenation: false,
            uniform_block: None,
        })
    }
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&spec.source_path)
            .map_err(|e| AdapterError::Transient(format!("source vanished: {e}")))?;
        f.seek(SeekFrom::Start(lease.resume_from)).unwrap();
        let want = (lease.end - lease.resume_from) as usize;
        let mut buf = vec![0u8; want];
        f.read_exact(&mut buf)
            .map_err(|e| AdapterError::Transient(format!("short read: {e}")))?;
        progress.add(want as u64);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: want as u64,
            adapter_token: Some(format!("part-{}", lease.idx + 1)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("scripted S3: whole-object confirm".into()),
        })
    }

    // THE #57 CHANNEL — not yet on the trait (the RED): the manager must
    // ask the ADAPTER for its shaped probe, never string-match "tus".
    async fn probe_link(
        &self,
        _spec: &JobSpec,
        _k: u32,
        _sample_hint_bps: Option<f64>,
    ) -> Result<Option<LinkProfile>, AdapterError> {
        *self.probe_link_calls.lock().unwrap() += 1;
        let script = self
            .script
            .lock()
            .unwrap()
            .take()
            .expect("probe_link called at most once per job");
        match script {
            ProbeScript::Profile(p) => Ok(Some(p)),
            ProbeScript::Silent => Ok(None),
            ProbeScript::Down(why) => Err(AdapterError::Transient(why.into())),
        }
    }

    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn per_flow_profile() -> LinkProfile {
    // 64 KiB single, 256 KiB aggregate — the measurement says "multiply",
    // but the RECOMMENDED PIECE (64 KiB) is under the S3 lane floor: the
    // pool must floor it (the 5 MiB multipart minimum is structural).
    LinkProfile {
        single_bps: 64.0 * KIB as f64,
        k_bps: 256.0 * KIB as f64,
        k: 4,
        class: LinkClass::PerFlowCap,
        recommended_piece_bytes: 64 * KIB,
        recommended_fleet: 4,
    }
}

fn write_source(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("src.bin");
    let src: Vec<u8> = (0..SPAN as usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &src).unwrap();
    path
}

fn events(db: &std::path::Path, job: &str) -> Vec<(String, String)> {
    SessionJournal::open(db)
        .unwrap()
        .events_for(job)
        .unwrap()
        .into_iter()
        .map(|(_, k, p)| (k, p))
        .collect()
}

fn count(evs: &[(String, String)], kind: &str) -> usize {
    evs.iter().filter(|(k, _)| k == kind).count()
}

async fn drain_to_done(
    m: &JobManager,
    job: &str,
    db: &std::path::Path,
) -> ium_core::journal::JobRecord {
    let done = tokio::time::timeout(Duration::from_secs(60), m.run_job(job))
        .await
        .expect("job must finish inside the drill budget")
        .expect("run");
    if done.state != JobState::Done {
        for (k, p) in events(db, job).into_iter().rev().take(40) {
            eprintln!("[drill] {k}: {p}");
        }
    }
    done
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_link_probe_journals_once_replans_and_holds_the_lane_floor() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("IUM_LINK_PROBE", "1");
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(dir.path());
    let db = dir.path().join("journal.db");
    let mut m = JobManager::with_journal(SessionJournal::open(&db).unwrap()).unwrap();
    m.register_adapter(std::sync::Arc::new(ScriptedS3::new(ProbeScript::Profile(
        per_flow_profile(),
    ))));
    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "s3_multipart".into(),
        endpoint: "http://127.0.0.1:1/bucket-that-does-not-exist".into(),
    };
    let rec = m.create_job(&spec, 1).unwrap(); // born as ONE 12 MiB chunk
    let done = drain_to_done(&m, &rec.id, &db).await;
    assert_eq!(done.state, JobState::Done, "the job must still finish");

    let evs = events(&db, &rec.id);
    // Journal-once: exactly one measured profile.
    assert_eq!(count(&evs, "link_profile"), 1, "journal-once");
    assert_eq!(count(&evs, "link_profile_skipped"), 0);
    // The planner OBEYS: the replan is journaled with the recommendation.
    let replans: Vec<&(String, String)> =
        evs.iter().filter(|(k, _)| k == "replan_measured").collect();
    assert_eq!(replans.len(), 1, "the measured piece replanned the pool");
    let payload: serde_json::Value = serde_json::from_str(&replans[0].1).unwrap();
    assert_eq!(payload["recommended_piece"], 64 * KIB);
    // THE LANE FLOOR: 64 KiB pieces are IMPOSSIBLE on S3 (5 MiB multipart
    // minimum) — the pool floored the recommendation to >= CHUNK_MIN.
    let chunks = SessionJournal::open(&db)
        .unwrap()
        .load_chunks(&rec.id)
        .unwrap();
    assert!(chunks.len() >= 2, "the pool re-shaped (before: 1 chunk)");
    for c in &chunks {
        let span = c.end - c.begin;
        assert!(
            span >= 5 * MIB,
            "S3 lane floor held: chunk span {span} < 5 MiB is a wire impossibility"
        );
    }
    assert_eq!(
        chunks.iter().map(|c| c.end - c.begin).sum::<u64>(),
        SPAN,
        "the re-plan still covers the whole source"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_link_probe_failure_is_a_lost_measurement_never_a_lost_job() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("IUM_LINK_PROBE", "1");
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(dir.path());
    let db = dir.path().join("journal.db");
    let mut m = JobManager::with_journal(SessionJournal::open(&db).unwrap()).unwrap();
    m.register_adapter(std::sync::Arc::new(ScriptedS3::new(ProbeScript::Down(
        "probe leg: connection refused",
    ))));
    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "s3_multipart".into(),
        endpoint: "http://127.0.0.1:1/bucket".into(),
    };
    let rec = m.create_job(&spec, 2).unwrap(); // 2 x 6 MiB — the planned shape
    let done = drain_to_done(&m, &rec.id, &db).await;
    assert_eq!(
        done.state,
        JobState::Done,
        "a failed probe is NOT a lost job"
    );

    let evs = events(&db, &rec.id);
    assert_eq!(count(&evs, "link_profile"), 0, "no fabricated profile");
    assert_eq!(
        count(&evs, "link_profile_skipped"),
        1,
        "the skip is journaled"
    );
    let skips: Vec<&(String, String)> = evs
        .iter()
        .filter(|(k, _)| k == "link_profile_skipped")
        .collect();
    let payload: serde_json::Value = serde_json::from_str(&skips[0].1).unwrap();
    assert!(
        payload["reason"]
            .as_str()
            .unwrap()
            .contains("connection refused"),
        "the skip carries the probe's own reason"
    );
    assert_eq!(
        count(&evs, "replan_measured"),
        0,
        "no measurement, no replan"
    );
    let chunks = SessionJournal::open(&db)
        .unwrap()
        .load_chunks(&rec.id)
        .unwrap();
    assert_eq!(chunks.len(), 2, "the original plan rode the job");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lane_without_a_shaped_prober_is_journal_silent() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("IUM_LINK_PROBE", "1");
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(dir.path());
    let db = dir.path().join("journal.db");
    let mut m = JobManager::with_journal(SessionJournal::open(&db).unwrap()).unwrap();
    m.register_adapter(std::sync::Arc::new(ScriptedS3::new(ProbeScript::Silent)));
    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "s3_multipart".into(),
        endpoint: "http://127.0.0.1:1/bucket".into(),
    };
    let rec = m.create_job(&spec, 2).unwrap();
    let done = drain_to_done(&m, &rec.id, &db).await;
    assert_eq!(done.state, JobState::Done);

    // Ok(None) = pre-#57 behavior, byte-identical: NO probe journal noise.
    let evs = events(&db, &rec.id);
    assert_eq!(count(&evs, "link_profile"), 0);
    assert_eq!(count(&evs, "link_profile_skipped"), 0);
    assert_eq!(count(&evs, "replan_measured"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_probe_is_asked_only_when_the_opt_in_is_set() {
    // The default-off face, S3-shaped lane: the adapter is NEVER asked.
    let _env = ENV_LOCK.lock().await;
    std::env::remove_var("IUM_LINK_PROBE");
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(dir.path());
    let db = dir.path().join("journal.db");
    let mut m = JobManager::with_journal(SessionJournal::open(&db).unwrap()).unwrap();
    let a = std::sync::Arc::new(ScriptedS3::new(ProbeScript::Silent));
    m.register_adapter(a.clone());
    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "s3_multipart".into(),
        endpoint: "http://127.0.0.1:1/bucket".into(),
    };
    let rec = m.create_job(&spec, 2).unwrap();
    let done = drain_to_done(&m, &rec.id, &db).await;
    assert_eq!(done.state, JobState::Done);
    assert_eq!(*a.probe_link_calls.lock().unwrap(), 0, "never asked");
    // restore the opt-in for sibling tests in this process
    std::env::set_var("IUM_LINK_PROBE", "1");
}
