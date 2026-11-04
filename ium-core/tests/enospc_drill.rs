//! ENOSPC drills (issue #34) — what the engine does when TRUTH cannot be
//! written (ADR-002: the journal is the only truth).
//!
//! This file holds the INJECTED-failure shapes (`SessionJournal::
//! drill_fail_writes` — the fault-injection technique of SQLite's own test
//! suite), which need no process-wide state and run on every platform:
//!
//! * [`enospc_mid_transfer_parks_paused_loud_and_resumes_byte_exact`] — a
//!   mid-transfer journal write fails exactly once while everything after
//!   it succeeds (the transient-ENOSPC shape). Proves the honest park:
//!   `Paused` + `journal_write_failed` atomically journaled, loud error to
//!   the caller, no false Done, resume byte-exact.
//! * [`half_transition_leaves_no_state_without_its_event`] — a failure
//!   between a transition's two writes. The invariant: a state row never
//!   lands without its event.
//!
//! The REAL-OS disk-full shape (`RLIMIT_FSIZE` + SIGXFSZ — writes past the
//! cap return EFBIG, SQLite surfaces SystemIoFailure, the exact family
//! these injected errors replicate) lives in `enospc_rlimit_drill.rs`:
//! rlimit is process-wide, so it must own its test process.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

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
/// 12 MiB → the planner (floor(size / 5 MiB) capped by fleet) makes 2 chunks
/// of 6 MiB: one confirms honestly, one is mid-flight when the disk fills.
const SOURCE_SIZE: u64 = 12 * MIB;

fn pattern_byte(offset: u64) -> u8 {
    (offset % 251) as u8
}

fn make_source(dir: &std::path::Path) -> (std::path::PathBuf, Arc<Vec<u8>>) {
    let path = dir.join("source.bin");
    let mut bytes = Vec::with_capacity(SOURCE_SIZE as usize);
    for off in 0..SOURCE_SIZE {
        bytes.push(pattern_byte(off));
    }
    std::fs::write(&path, &bytes).unwrap();
    (path, Arc::new(bytes))
}

/// The "server": bytes it has acknowledged, keyed by absolute offset.
/// Shared across manager generations so a resumed drive lands on the same
/// object and `verify` can prove byte-exactness.
type Store = Arc<Mutex<Vec<u8>>>;

fn new_store() -> Store {
    Arc::new(Mutex::new(vec![0u8; SOURCE_SIZE as usize]))
}

/// Deterministic stand-in server. Uploads are SERIALIZED by a mutex so the
/// write ordering around the armed failure is stable. When the weak manager
/// handle resolves, the adapter arms the journal's drill hook (or the real
/// rlimit) at the chosen call boundary — the same place a real server would
/// observe the disk dying underneath a confirm.
struct DrillAdapter {
    store: Store,
    source: Arc<Vec<u8>>,
    upload_gate: Mutex<()>,
    calls: AtomicU32,
    /// Arm (at upload_chunk entry, 1-based call number). 0 = never.
    arm_at_call: AtomicU32,
    /// Arm inside `probe` (the boundary between the two transitions), with
    /// `(skip, fail)` semantics — T3 arms `(1, 1)` so the failure lands
    /// BETWEEN a transition's two writes (state row committed, event lost).
    arm_in_probe: bool,
    arm_skip: u32,
    arm_fail: u32,
    /// Armed ONCE.
    armed: AtomicU32,
    mgr: OnceLock<Weak<JobManager>>,
}

impl DrillAdapter {
    fn new(store: Store, source: Arc<Vec<u8>>) -> Self {
        Self {
            store,
            source,
            upload_gate: Mutex::new(()),
            calls: AtomicU32::new(0),
            arm_at_call: AtomicU32::new(0),
            arm_in_probe: false,
            arm_skip: 0,
            arm_fail: 1,
            armed: AtomicU32::new(0),
            mgr: OnceLock::new(),
        }
    }

    fn arm_once(&self) {
        if self.armed.swap(1, Ordering::SeqCst) == 1 {
            return;
        }
        // Injected flavor: after `arm_skip` honest writes, fail `arm_fail`
        // writes; everything after lands (space "recovered" — the
        // transient-ENOSPC shape).
        let Some(m) = self.mgr.get().and_then(Weak::upgrade) else {
            panic!("drill adapter: manager handle gone before arming");
        };
        m.journal().drill_fail_writes(self.arm_skip, self.arm_fail);
    }
}

#[async_trait]
impl TransferAdapter for DrillAdapter {
    fn name(&self) -> &'static str {
        "drill_enospc"
    }

    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }

    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        if self.arm_in_probe {
            self.arm_once();
        }
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
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
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.arm_at_call.load(Ordering::SeqCst) == n {
            self.arm_once();
        }
        // Serialize the fleet: a stable write order around the armed failure.
        let _gate = self.upload_gate.lock().unwrap();
        let seg = lease.end - lease.resume_from;
        let mut store = self.store.lock().unwrap();
        // resume_from is the ABSOLUTE offset (integration.rs parity).
        for i in 0..seg {
            let abs = lease.resume_from + i;
            store[abs as usize] = self.source[abs as usize];
        }
        drop(store);
        progress.add(seg);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: seg,
            adapter_token: None,
        })
    }

    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let store = self.store.lock().unwrap();
        let bad = (0..SOURCE_SIZE).find(|&off| store[off as usize] != pattern_byte(off));
        match bad {
            None => Ok(VerifyReport {
                ok: true,
                detail: Some(format!("{SOURCE_SIZE} bytes byte-exact")),
            }),
            Some(off) => Ok(VerifyReport {
                ok: false,
                detail: Some(format!("byte mismatch at offset {off}")),
            }),
        }
    }

    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn spec_for(source: &std::path::Path) -> JobSpec {
    JobSpec {
        source_path: source.to_string_lossy().into_owned(),
        source_size: SOURCE_SIZE,
        adapter: "drill_enospc".into(),
        endpoint: "drill://enospc".into(),
    }
}

fn manager_with_adapter(
    dir: &std::path::Path,
    store: &Store,
    source: &Arc<Vec<u8>>,
    arm_at_call: u32,
    arm_in_probe: bool,
    arm_skip: u32,
) -> (Arc<JobManager>, Arc<DrillAdapter>) {
    let journal = SessionJournal::open(dir.join("journal.db")).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    let adapter = Arc::new(DrillAdapter {
        store: Arc::clone(store),
        source: Arc::clone(source),
        upload_gate: Mutex::new(()),
        calls: AtomicU32::new(0),
        arm_at_call: AtomicU32::new(arm_at_call),
        arm_in_probe,
        arm_skip,
        arm_fail: 1,
        armed: AtomicU32::new(0),
        mgr: OnceLock::new(),
    });
    m.register_adapter(Arc::clone(&adapter) as Arc<dyn TransferAdapter>);
    let m = Arc::new(m);
    adapter
        .mgr
        .set(Arc::downgrade(&m))
        .expect("manager handle set once");
    (m, adapter)
}

fn events(m: &JobManager, id: &str) -> Vec<(i64, String, String)> {
    m.journal().events_for(id).unwrap()
}

fn job_state(m: &JobManager, id: &str) -> JobState {
    m.journal().load_job(id).unwrap().unwrap().state
}

fn has_event(events: &[(i64, String, String)], kind: &str) -> bool {
    events.iter().any(|(_, k, _)| k == kind)
}

/// The invariant the half-transition drill pins: a job whose ROW says
/// `Running` must have a `state` event that says so — a state without its
/// trail is silent corruption (ADR-002).
fn assert_running_has_state_event(m: &JobManager, id: &str) {
    if job_state(m, id) != JobState::Running {
        return;
    }
    let to_running = events(m, id).iter().any(|(_, k, payload)| {
        k == "state"
            && serde_json::from_str::<serde_json::Value>(payload)
                .ok()
                .and_then(|v| v.get("to").and_then(|t| t.as_str()).map(str::to_string))
                .as_deref()
                == Some("running")
    });
    assert!(
        to_running,
        "journal says Running with NO state event — half-transition corruption"
    );
}

/// T1 — injected single mid-transfer write failure (transient ENOSPC — the
/// common real shape: space frees again while the failure surfaces). The
/// honest park MUST land: Paused + journal_write_failed, loud Err, no false
/// Done; resume completes byte-exact.
#[tokio::test]
async fn enospc_mid_transfer_parks_paused_loud_and_resumes_byte_exact() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("t1");
    std::fs::create_dir(&dir).unwrap();
    let (path, source) = make_source(&dir);
    let path = Arc::new(path);
    let store = new_store();
    let (m, _a) = manager_with_adapter(&dir, &store, &source, 2, false, 0);
    let rec = m.create_job(&spec_for(&path), 2).unwrap();

    let err = m.run_job(&rec.id).await.unwrap_err();
    assert!(
        err.to_string().contains("journal error"),
        "failure must be LOUD through the journal error channel: {err}"
    );
    // The honest park (RED at HEAD: state dangles at Running).
    assert_eq!(
        job_state(&m, &rec.id),
        JobState::Paused,
        "mid-transfer ENOSPC must park the job Paused, honestly"
    );
    let ev = events(&m, &rec.id);
    assert!(
        has_event(&ev, "journal_write_failed"),
        "the park must carry a journal_write_failed event: {ev:?}"
    );
    assert!(
        !has_event(&ev, "verified") && job_state(&m, &rec.id) != JobState::Done,
        "no false Done: verify-before-declare survives a full disk"
    );

    // Space "freed" (the injected failure already disarmed): a FRESH
    // manager over the same journal replays truthfully and completes.
    let (m2, _a2) = manager_with_adapter(&dir, &store, &source, 0, false, 0);
    let rec2 = m2.run_job(&rec.id).await.unwrap();
    assert_eq!(rec2.state, JobState::Done);
    assert!(m2
        .journal()
        .events_for(&rec.id)
        .unwrap()
        .iter()
        .any(|(_, k, _)| k == "verified"));
    let a2 = DrillAdapter::new(Arc::clone(&store), Arc::clone(&source));
    let report = a2.verify(&spec_for(&path)).await.unwrap();
    assert!(report.ok, "resume after ENOSPC must complete byte-exact");
}

/// T3 — half-transition: one injected failure BETWEEN a transition's two
/// writes (armed in the probe, skipping the first write of the running
/// transition). The pinned invariant: a job whose row says `Running` MUST
/// have a `state` event saying so — a state without its trail is the
/// silent-corruption shape of a disk-full landing mid-transition.
#[tokio::test]
async fn half_transition_leaves_no_state_without_its_event() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("t3");
    std::fs::create_dir(&dir).unwrap();
    let (path, source) = make_source(&dir);
    let path = Arc::new(path);
    let store = new_store();
    let (m, _a) = manager_with_adapter(&dir, &store, &source, 0, true, 2);
    let rec = m.create_job(&spec_for(&path), 2).unwrap();

    let err = m.run_job(&rec.id).await.unwrap_err();
    assert!(
        err.to_string().contains("journal error"),
        "half-transition must surface loudly: {err}"
    );
    assert_running_has_state_event(&m, &rec.id);
    let ev = events(&m, &rec.id);
    assert!(!has_event(&ev, "verified"));

    // Recovery: disarm (the single injected failure was consumed) and
    // re-drive — no corruption, byte-exact Done.
    let (m2, _a2) = manager_with_adapter(&dir, &store, &source, 0, false, 0);
    let rec2 = m2.run_job(&rec.id).await.unwrap();
    assert_eq!(rec2.state, JobState::Done);
    let a2 = DrillAdapter::new(Arc::clone(&store), Arc::clone(&source));
    assert!(a2.verify(&spec_for(&path)).await.unwrap().ok);
}
