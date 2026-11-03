//! REAL-OS disk-full drill (issue #34) — the shape the injected drill in
//! `enospc_drill.rs` replicates.
//!
//! Mechanism: `RLIMIT_FSIZE` with SIGXFSZ ignored. Any write past the soft
//! cap fails with EFBIG — the same syscall-level failure a full filesystem
//! produces (SQLite surfaces it in the SystemIoFailure/SQLITE_FULL family,
//! identical to what the loopback-image drill sees on sudo runners). The
//! cap is armed MID-TRANSFER by the mock adapter and STAYS ARMED: the
//! honest park itself cannot be journaled. The drill pins what that means:
//!
//! * the error stays LOUD (journal error channel, never swallowed);
//! * the row keeps its last HONEST state — Running (the drive really did
//!   die mid-round), never a lie, never Done, never Failed;
//! * nothing corrupts: a state row always has its event, the journal stays
//!   replayable;
//! * after the operator frees the space, recovery completes byte-exact.
//!
//! This file owns its test PROCESS (cargo runs each test binary separately):
//! rlimit and signal dispositions are process-wide and must not leak into
//! the other drills.

use std::path::PathBuf;
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
const SOURCE_SIZE: u64 = 12 * MIB; // 2 chunks of 6 MiB

fn pattern_byte(offset: u64) -> u8 {
    (offset % 251) as u8
}

type Store = Arc<Mutex<Vec<u8>>>;

fn new_store() -> Store {
    Arc::new(Mutex::new(vec![0u8; SOURCE_SIZE as usize]))
}

/// Stand-in server; arms the real rlimit at the chosen upload call and
/// leaves it armed (the disk stays full until the "operator" frees it).
struct RlimitAdapter {
    store: Store,
    source: Arc<Vec<u8>>,
    upload_gate: Mutex<()>,
    calls: AtomicU32,
    arm_at_call: AtomicU32,
    armed: AtomicU32,
    dir: PathBuf,
    mgr: OnceLock<Weak<JobManager>>,
}

impl RlimitAdapter {
    fn arm_once(&self) {
        if self.armed.swap(1, Ordering::SeqCst) == 1 {
            return;
        }
        let total: u64 = ["journal.db", "journal.db-wal", "journal.db-shm"]
            .iter()
            .map(|f| {
                std::fs::metadata(self.dir.join(f))
                    .map(|m| m.len())
                    .unwrap_or(0)
            })
            .sum();
        // +16 B of headroom: the very next WAL frame append (≥ 24 B header)
        // must fail — the deterministic trip point.
        let cap = total + 16;
        let lim = libc::rlimit {
            rlim_cur: cap,
            rlim_max: 2 * 1024 * 1024 * 1024, // hard ceiling stays generous
        };
        let rc = unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &lim) };
        assert_eq!(rc, 0, "setrlimit failed");
        eprintln!(
            "[rlimit-drill] disk-full armed at {} bytes (journal db+wal+shm + 16)",
            cap
        );
    }
}

#[async_trait]
impl TransferAdapter for RlimitAdapter {
    fn name(&self) -> &'static str {
        "drill_enospc_rlimit"
    }

    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }

    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            max_parallel: None,
            session_token: None,
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
        let _gate = self.upload_gate.lock().unwrap();
        let seg = lease.end - lease.resume_from;
        let mut store = self.store.lock().unwrap();
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

#[cfg(unix)]
#[tokio::test]
async fn real_disk_full_is_loud_truthful_and_recoverable() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("t2");
    std::fs::create_dir(&dir).unwrap();

    // Writes past the cap must return EFBIG to the caller, not kill the
    // process (SIGXFSZ's default disposition is terminate).
    unsafe {
        libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
    }

    let source_path = dir.join("source.bin");
    let mut bytes = Vec::with_capacity(SOURCE_SIZE as usize);
    for off in 0..SOURCE_SIZE {
        bytes.push(pattern_byte(off));
    }
    std::fs::write(&source_path, &bytes).unwrap();
    let source = Arc::new(bytes);
    let store = new_store();

    let journal = SessionJournal::open(dir.join("journal.db")).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    let adapter = Arc::new(RlimitAdapter {
        store: Arc::clone(&store),
        source: Arc::clone(&source),
        upload_gate: Mutex::new(()),
        calls: AtomicU32::new(0),
        arm_at_call: AtomicU32::new(2),
        armed: AtomicU32::new(0),
        dir: dir.clone(),
        mgr: OnceLock::new(),
    });
    m.register_adapter(Arc::clone(&adapter) as Arc<dyn TransferAdapter>);
    let m = Arc::new(m);
    adapter
        .mgr
        .set(Arc::downgrade(&m))
        .expect("manager handle set once");

    let spec = JobSpec {
        source_path: source_path.to_string_lossy().into_owned(),
        source_size: SOURCE_SIZE,
        adapter: "drill_enospc_rlimit".into(),
        endpoint: "drill://enospc-rlimit".into(),
    };
    let rec = m.create_job(&spec, 2).unwrap();

    // -- the disk fills mid-transfer ------------------------------------
    let err = m.run_job(&rec.id).await.unwrap_err();
    let loud = err.to_string().to_lowercase();
    assert!(
        loud.contains("journal") || loud.contains("disk i/o") || loud.contains("sqlite"),
        "real EFBIG must surface loudly through the error channel, got: {err}"
    );

    // Truthful: the drive died mid-round with nothing writable — the row
    // keeps its last HONEST state. Never a lie, never Done, never Failed
    // (nothing FAILED except the disk; no retry budget was exhausted).
    let state = m.journal().load_job(&rec.id).unwrap().unwrap().state;
    assert_ne!(state, JobState::Done, "no false Done under ENOSPC");
    assert_ne!(state, JobState::Failed, "ENOSPC is not a transfer failure");
    let ev = m.journal().events_for(&rec.id).unwrap();
    assert!(
        !ev.iter().any(|(_, k, _)| k == "verified"),
        "no verify-before-declare bypass under ENOSPC"
    );
    // State-implies-event holds even here.
    if state == JobState::Running {
        assert!(
            ev.iter().any(|(_, k, payload)| k == "state"
                && serde_json::from_str::<serde_json::Value>(payload)
                    .ok()
                    .and_then(|v| { v.get("to").and_then(|t| t.as_str()).map(str::to_string) })
                    .as_deref()
                    == Some("running")),
            "Running row without its state event — half-transition corruption under real EFBIG"
        );
    }

    // -- the operator frees space ----------------------------------------
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    unsafe {
        libc::getrlimit(libc::RLIMIT_FSIZE, &mut lim);
    }
    lim.rlim_cur = lim.rlim_max; // raising SOFT up to HARD is always allowed
    unsafe {
        libc::setrlimit(libc::RLIMIT_FSIZE, &lim);
    }

    // -- recovery: fresh manager over the SAME journal, byte-exact Done ---
    let journal2 = SessionJournal::open(dir.join("journal.db")).unwrap();
    let mut m2 = JobManager::with_journal(journal2).unwrap();
    let a2 = Arc::new(RlimitAdapter {
        store: Arc::clone(&store),
        source: Arc::clone(&source),
        upload_gate: Mutex::new(()),
        calls: AtomicU32::new(0),
        arm_at_call: AtomicU32::new(0),
        armed: AtomicU32::new(0),
        dir: dir.clone(),
        mgr: OnceLock::new(),
    });
    m2.register_adapter(Arc::clone(&a2) as Arc<dyn TransferAdapter>);
    let rec2 = m2.run_job(&rec.id).await.unwrap();
    assert_eq!(rec2.state, JobState::Done, "recovery after freeing space");
    let report = a2.verify(&spec).await.unwrap();
    assert!(report.ok, "recovery must be byte-exact: {report:?}");
}
