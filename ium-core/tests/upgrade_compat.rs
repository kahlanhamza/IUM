//! The upgrade-path pin (#96): a journal written by the **previous release**
//! (v0.2.0) must open and resume byte-exact at HEAD — the N-1 → N contract.
//!
//! The fixture `tests/fixtures/upgrade_v020_journal.db` was produced by the
//! v0.2.0 tag's own code (see `scripts/upgrade_fixture.sh` +
//! `scripts/upgrade_fixture/gen_upgrade_fixture.rs`): the real v0.2.0
//! `JobManager` drove a 12 MiB job to mid-flight and was dropped mid-call —
//! the in-process twin of a SIGKILL'd v0.2.0. Fixture sha256:
//! d393fb592bf9dd81befdf88e03b1e60bc6474fad4229ce0af1665a89a48fddec
//!
//! What v0.2.0 left on disk (printed by the generator, pinned below):
//! ```text
//! job 01M71E5TRWNCFZC9RJN54WPRA7: Running, 12582912 B, mock_scripted,
//!                                 loopback://upgrade-fixture
//! chunk 0 Uploaded confirmed=3145728 [0,3145728)       token part:1
//! chunk 1 Uploaded confirmed=3145728 [3145728,6291456) token part:2
//! chunk 2 Uploaded confirmed=3145728 [6291456,9437184) token part:3
//! chunk 3 Leased  confirmed=1572864 [9437184,12582912) token None   ← mid-body
//! events: job_created, state, probe, replan_cap, state, chunk_confirmed ×4
//!         (v0.2.0-era kinds only — no link_profile, no leases_reclaimed)
//! ```
//!
//! The pins:
//! 1. `user_version` is 1 before AND after HEAD's migrate() — **this is the
//!    MIGRATIONS tripwire**: when a v2 migration ever lands, this assert
//!    fails and this lane MUST be extended with a v2-era fixture. Never bump
//!    the expected number without a new fixture from the previous release.
//! 2. The rows read back exactly what v0.2.0 wrote (journal-truth round trip).
//! 3. Resume moves ONLY the missing bytes: exactly one adapter call, idx 3,
//!    `resume_from == 11_010_048` (the half-confirmed frontier) — chunks 0–2
//!    and the confirmed half are NEVER re-sent. Zero re-upload across the
//!    upgrade is asserted over the wire, not promised.
//! 4. The source is regenerated from the generator's deterministic LCG and
//!    the received bytes are compared against it — byte-exact.
//! 5. A future event kind (one HEAD never wrote) is tolerated by the reader
//!    end-to-end: appended to a copy, the resume still reaches Done.

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::{ChunkState, Lease};
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The fixture's absolute source path row (v0.2.0 wrote it; the test
/// regenerates the deterministic file there — /tmp is writable on every
/// POSIX CI runner; the row itself is NEVER rewritten).
const ROW_SOURCE_PATH: &str = "/tmp/ium-upgrade-fixture/upgrade_v020_source.bin";
const SOURCE_SIZE: u64 = 12 * 1024 * 1024;
const CHUNK: u64 = 3 * 1024 * 1024;
/// The half-confirmed frontier of chunk 3 — where v0.2.0 stopped.
const FRONTIER: u64 = 9_437_184 + CHUNK / 2; // 11_010_048

fn regenerate_source(path: &Path) -> Vec<u8> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create source dir");
    }
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut buf = Vec::with_capacity(SOURCE_SIZE as usize);
    while buf.len() < SOURCE_SIZE as usize {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        buf.push((state >> 33) as u8);
    }
    std::fs::write(path, &buf).expect("write regenerated source");
    buf
}

/// The HEAD-side stand-in for the fixture's adapter key. Records every call
/// and assembles the bytes it moves, so zero-re-upload and byte-exactness
/// are both asserted from the same log.
struct RecordingAdapter {
    cap: Capability,
    source: std::sync::Mutex<Vec<u8>>,
    calls: std::sync::Mutex<Vec<(u32, u64)>>, // (idx, resume_from) per call
    moved: std::sync::atomic::AtomicU64,
    received: std::sync::Mutex<Vec<u8>>, // bytes moved in THIS process, in order
}

impl RecordingAdapter {
    fn new(source: Vec<u8>) -> Self {
        Self {
            cap: Capability::new(Capability::RESUMABLE | Capability::DYNAMIC_CHUNKS),
            source: std::sync::Mutex::new(source),
            calls: std::sync::Mutex::new(Vec::new()),
            moved: std::sync::atomic::AtomicU64::new(0),
            received: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl TransferAdapter for RecordingAdapter {
    fn name(&self) -> &'static str {
        "mock_scripted"
    }
    fn capabilities(&self) -> Capability {
        self.cap
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(CHUNK),
            min_chunk_bytes: None,
            max_parallel: None,
            session_token: Some("head-recording-session".into()),
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
        self.calls
            .lock()
            .unwrap()
            .push((lease.idx, lease.resume_from));
        let span = (lease.end - lease.resume_from) as usize;
        let src = self.source.lock().unwrap();
        let bytes = src[lease.resume_from as usize..lease.end as usize].to_vec();
        assert_eq!(
            bytes.len(),
            span,
            "the regenerated source must cover the lease"
        );
        self.moved
            .fetch_add(span as u64, std::sync::atomic::Ordering::SeqCst);
        self.received.lock().unwrap().extend_from_slice(&bytes);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span as u64,
            adapter_token: Some(format!("head:{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("upgrade-compat stand-in: assembled bytes pinned by the test".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn fixture_copy(tag: &str) -> (tempdir::Temp, PathBuf) {
    let tmp = tempdir::Temp::new(tag);
    let db = tmp.path.join("journal.db");
    std::fs::copy("tests/fixtures/upgrade_v020_journal.db", &db)
        .expect("fixture present (committed at HEAD)");
    (tmp, db)
}

/// Minimal writable-tempdir helper (the suite's common helpers are
/// video-focused; this lane needs only a directory).
mod tempdir {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    pub struct Temp {
        pub path: PathBuf,
    }
    static N: AtomicU32 = AtomicU32::new(0);
    impl Temp {
        pub fn new(tag: &str) -> Self {
            let n = N.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "ium-upgrade-{}-{}-{}",
                tag,
                std::process::id(),
                n
            ));
            std::fs::create_dir_all(&path).expect("tempdir");
            Self { path }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn v020_journal_opens_and_resumes_byte_exact() {
    let src_bytes = regenerate_source(Path::new(ROW_SOURCE_PATH));
    assert_eq!(src_bytes.len() as u64, SOURCE_SIZE);

    let (_tmp, db) = fixture_copy("resume");

    // The tripwire: v1 today. Bumping this number WITHOUT a new N-1 fixture
    // from the previous release breaks the upgrade contract — never do it.
    let uv = rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert_eq!(
        uv, 1,
        "a v2 migration landed — extend this lane with a v2 fixture FIRST"
    );

    // The rows read back exactly what v0.2.0 wrote.
    let j = SessionJournal::open(&db).expect("HEAD opens the v0.2.0 journal");
    let job = j
        .load_job("01M71E5TRWNCFZC9RJN54WPRA7")
        .expect("load")
        .expect("the v0.2.0 job row survives");
    assert_eq!(job.state, JobState::Running);
    assert_eq!(job.source_size, SOURCE_SIZE);
    assert_eq!(job.adapter, "mock_scripted");
    assert_eq!(job.endpoint, "loopback://upgrade-fixture");
    assert_eq!(job.source_path, ROW_SOURCE_PATH);
    let chunks = j.load_chunks(&job.id).expect("chunks");
    let expect: [(u32, ChunkState, u64, u64, u64); 4] = [
        (0, ChunkState::Uploaded, CHUNK, 0, CHUNK),
        (1, ChunkState::Uploaded, CHUNK, CHUNK, 2 * CHUNK),
        (2, ChunkState::Uploaded, CHUNK, 2 * CHUNK, 3 * CHUNK),
        (3, ChunkState::Leased, CHUNK / 2, 3 * CHUNK, SOURCE_SIZE),
    ];
    for (idx, state, confirmed, begin, end) in expect {
        let c = chunks.iter().find(|c| c.idx == idx).expect("chunk row");
        assert_eq!(c.state, state, "chunk {idx} state");
        assert_eq!(c.bytes_confirmed, confirmed, "chunk {idx} confirmed");
        assert_eq!((c.begin, c.end), (begin, end), "chunk {idx} span");
    }
    let kinds: Vec<String> = j
        .events_for(&job.id)
        .expect("events")
        .into_iter()
        .map(|(_, k, _)| k)
        .collect();
    assert_eq!(
        kinds[0], "job_created",
        "v0.2.0's first event kind round-trips"
    );
    assert_eq!(kinds.iter().filter(|k| **k == "chunk_confirmed").count(), 4);
    assert!(
        !kinds
            .iter()
            .any(|k| k == "link_profile" || k == "leases_reclaimed"),
        "the fixture must carry NO HEAD-era kinds (it is an N-1 artifact)"
    );
    drop(j);

    // Resume at HEAD: only the missing bytes move.
    let adapter = Arc::new(RecordingAdapter::new(src_bytes));
    let mut mgr = JobManager::open(&db).expect("manager over the upgraded journal");
    mgr.register_adapter(adapter.clone());
    let done = mgr.run_job(&job.id).await.expect("resume to completion");
    assert_eq!(done.state, JobState::Done);

    let calls = adapter.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![(3u32, FRONTIER)],
        "exactly ONE call: chunk 3 from the confirmed frontier — chunks 0-2 and \
         the confirmed half are never re-sent across the upgrade"
    );
    assert_eq!(
        adapter.moved.load(std::sync::atomic::Ordering::SeqCst),
        SOURCE_SIZE - FRONTIER,
        "the whole upgrade re-sent exactly the missing bytes"
    );
    let expected_tail = regenerate_source(Path::new(ROW_SOURCE_PATH))
        [FRONTIER as usize..SOURCE_SIZE as usize]
        .to_vec();
    assert_eq!(
        *adapter.received.lock().unwrap(),
        expected_tail,
        "byte-exact: the resumed leg carried the true source bytes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_future_event_kind_is_tolerated_end_to_end() {
    let src_bytes = regenerate_source(Path::new(ROW_SOURCE_PATH));
    let (_tmp, db) = fixture_copy("future-kind");

    // A kind HEAD never wrote — what release N+2 will leave behind when this
    // binary is the N-1. The reader must skip it honestly, never panic.
    {
        let j = SessionJournal::open(&db).expect("open");
        j.append_event(
            "01M71E5TRWNCFZC9RJN54WPRA7",
            "kind_from_the_future",
            &serde_json::json!({"opaque": {"nested": [1, 2, 3]}}),
        )
        .expect("append future kind");
    }

    let adapter = Arc::new(RecordingAdapter::new(src_bytes));
    let mut mgr = JobManager::open(&db).expect("manager tolerates the future kind");
    mgr.register_adapter(adapter.clone());
    let done = mgr
        .run_job("01M71E5TRWNCFZC9RJN54WPRA7")
        .await
        .expect("resume");
    assert_eq!(done.state, JobState::Done);
    assert_eq!(
        adapter.calls.lock().unwrap().len(),
        1,
        "still zero re-upload"
    );
}
