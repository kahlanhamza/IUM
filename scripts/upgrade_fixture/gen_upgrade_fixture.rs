//! Upgrade fixture generator — runs against the **v0.2.0** tag code ONLY.
//!
//! This file lives at HEAD in `scripts/upgrade_fixture/` for provenance and is
//! copied into a `git worktree` of `v0.2.0` by `scripts/upgrade_fixture.sh`.
//! It drives the real v0.2.0 `JobManager` (the previous release's own code
//! paths: `create_job` → planner → leases → confirms → counted retries) into
//! a genuine mid-flight state, then drops the drive future mid-call — the
//! exact journal shape a SIGKILL'd v0.2.0 leaves behind (async cancellation
//! at the pending upload is the honest in-process twin of kill -9):
//!
//!   job   : Running, adapter "mock_scripted", endpoint loopback://upgrade-fixture
//!   chunks: 0..2 fully Uploaded (server-confirmed), chunk 3 Leased with a
//!           partial `bytes_confirmed` (half), chunk 4+ Pending (planned)
//!   events: v0.2.0's authentic event stream — no HEAD-era kinds
//!
//! Env: IUM_UPGRADE_FIXTURE_DIR (where journal.db is written), loud skip
//! without it so the file is inert in any normal suite run.

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::{JobManager, JobSpec};

use std::path::PathBuf;
use std::sync::Arc;

/// 12 MiB deterministic source (LCG-seeded — same bytes on every machine).
const SOURCE_SIZE: u64 = 12 * 1024 * 1024;
/// Adapter-declared chunk cap → 4 chunks of 3 MiB.
const MAX_CHUNK: u64 = 3 * 1024 * 1024;

fn sha256_file(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(std::fs::read(path).expect("read source"));
    format!("{:x}", h.finalize())
}

fn write_deterministic_source(path: &std::path::Path) -> String {
    use std::io::Write;
    let mut f = std::fs::File::create(path).expect("create source");
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut buf = vec![0u8; 64 * 1024];
    let mut left = SOURCE_SIZE;
    while left > 0 {
        let n = buf.len().min(left as usize);
        for b in buf.iter_mut().take(n) {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (state >> 33) as u8;
        }
        f.write_all(&buf[..n]).expect("write source");
        left -= n as u64;
    }
    f.sync_all().expect("sync source");
    sha256_file(path)
}

struct MidFlightAdapter {
    cap: Capability,
}

#[async_trait::async_trait]
impl TransferAdapter for MidFlightAdapter {
    fn name(&self) -> &'static str {
        "mock_scripted"
    }
    fn capabilities(&self) -> Capability {
        self.cap
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(MAX_CHUNK),
            min_chunk_bytes: None,
            max_parallel: None,
            session_token: Some("v020-midflight-session".into()),
            concatenation: false,
            uniform_block: None,
        })
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        lease: &Lease,
        _progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let span = lease.end - lease.resume_from;
        if lease.idx == 3 && lease.resume_from == lease.begin {
            // First lease of the tail chunk: confirm HALF, then stop —
            // the NEXT lease hangs forever (the call that gets aborted).
            Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: span / 2,
                adapter_token: Some("part:4:half".into()),
            })
        } else if lease.idx == 3 {
            // The call the drive is dropped inside.
            std::future::pending::<()>().await;
            unreachable!()
        } else {
            Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: span,
                adapter_token: Some(format!("part:{}", lease.idx + 1)),
            })
        }
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        unreachable!("the aborted drive never reaches the finish gate")
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// Poll the journal until chunk 3 is half-confirmed AND re-leased (its second
/// call genuinely in flight), with a hard give-up so the generator can never
/// hang a session.
async fn wait_for_shape(mgr: &JobManager, job_id: &str) {
    for _ in 0..600 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let c3 = {
            let j = mgr.journal();
            let chunks = j.load_chunks(job_id).expect("load chunks");
            chunks.into_iter().find(|c| c.idx == 3).expect("chunk 3 planned")
        };
        let leased = matches!(c3.state, ium_core::chunk::ChunkState::Leased);
        if leased && c3.bytes_confirmed == MAX_CHUNK / 2 {
            // one beat so the hang is genuinely mid-call
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            return;
        }
    }
    panic!("the mid-flight shape never formed (30 s give-up)");
}

#[tokio::test(flavor = "multi_thread")]
async fn generate_v020_midflight_journal() {
    let Some(dir) = std::env::var_os("IUM_UPGRADE_FIXTURE_DIR") else {
        panic!("loud skip: set IUM_UPGRADE_FIXTURE_DIR (the generator never runs in a suite)");
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");

    // Deterministic source — its sha256 is printed for the HEAD-side pin
    // (the source is REGENERATED at resume time, never shipped).
    let source = dir.join("upgrade_v020_source.bin");
    let sha = write_deterministic_source(&source);

    let mut mgr = JobManager::open(dir.join("journal.db")).expect("open journal");
    mgr.register_adapter(Arc::new(MidFlightAdapter {
        cap: Capability::new(Capability::RESUMABLE | Capability::DYNAMIC_CHUNKS),
    }));

    let spec = JobSpec {
        source_path: source.to_string_lossy().into_owned(),
        source_size: SOURCE_SIZE,
        adapter: "mock_scripted".into(),
        endpoint: "loopback://upgrade-fixture".into(),
    };
    let job = mgr.create_job(&spec, 2).expect("create job");
    eprintln!("[fixture] job id {}", job.id);

    // Drive and shape-watcher race; dropping the drive future mid-poll is
    // the mid-call kill. `run_job` holds a shared borrow; so does the
    // watcher — both are reads of the manager. Inner scope so the pinned
    // future's borrow ends before the manager is dropped.
    {
        let job_id = job.id.clone();
        let drive = std::pin::pin!(async { mgr.run_job(&job_id).await });
        let mut drive = drive;
        let watcher = wait_for_shape(&mgr, &job.id);
        tokio::select! {
            _ = &mut drive => panic!("the drive finished before the mid-flight shape formed"),
            _ = watcher => {}
        }
        drop(drive); // ← the abort: the pending upload dies here, journal untouched after
    }

    // Fold the WAL so the fixture is ONE self-contained file.
    drop(mgr);
    let j = ium_core::journal::SessionJournal::open(dir.join("journal.db")).expect("reopen");
    j.checkpoint_wal().expect("checkpoint");

    // Print the pinned facts (they go into the HEAD test's header).
    let job = j.load_job(&job.id).expect("load").expect("job exists");
    eprintln!(
        "[fixture] job row: state={:?} size={} adapter={} endpoint={}",
        job.state, job.source_size, job.adapter, job.endpoint
    );
    for c in j.load_chunks(&job.id).expect("chunks") {
        eprintln!(
            "[fixture] chunk {} {:?} confirmed={} [{},{}) token={:?}",
            c.idx, c.state, c.bytes_confirmed, c.begin, c.end, c.adapter_token
        );
    }
    let kinds: Vec<String> = j
        .events_for(&job.id)
        .expect("events")
        .into_iter()
        .map(|(_, k, _)| k)
        .collect();
    eprintln!("[fixture] event kinds: {}", kinds.join(","));
    eprintln!("[fixture] source sha256 {sha}");
    eprintln!(
        "[fixture] DONE — journal.db at {}",
        dir.join("journal.db").display()
    );
}
