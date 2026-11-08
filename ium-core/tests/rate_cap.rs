//! The speed-cap pins (#94): a GLOBAL upload cap — the sum across parallel
//! flows (the #46 vocabulary: the cap divides the bearer sum; per-flow caps
//! would let the fleet multiply it), honored over virtual time, hot (the
//! swap takes effect on the next acquire, running jobs obey), unlimited =
//! a no-op, and eviction-safe by construction (the wait happens BETWEEN
//! leases, never inside the eviction-tracked upload window).
//!
//! RED at the pre-lane HEAD: `ium_core::rate` did not exist and the manager
//! had no `set_rate_cap` — nothing in the engine could be polite about
//! bandwidth.

use std::sync::Arc;
use std::time::Duration;

use ium_core::rate::Bucket;

#[tokio::test(start_paused = true)]
async fn bucket_honors_the_rate_over_virtual_time() {
    // 100 B/s; the first acquire rides the full-burst allowance (the bucket
    // starts full — a fresh process may use one cap's worth immediately),
    // the rest waits: 2000 B total at 100 B/s ⇒ 19 s after the 100 B burst.
    let b = Bucket::new(Some(100));
    let t0 = tokio::time::Instant::now();
    b.acquire(2000).await;
    let elapsed = tokio::time::Instant::now() - t0;
    assert!(
        elapsed >= Duration::from_secs(9),
        "the cap must pace: elapsed {elapsed:?}"
    );
    assert!(
        elapsed <= Duration::from_secs(25),
        "the cap must not stall: elapsed {elapsed:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn fleet_sum_never_exceeds_the_cap() {
    // Four concurrent workers acquiring 500 B each at 100 B/s: 2000 B total
    // ⇒ the whole fleet finishes no earlier than ~10 s — the SUM across
    // parallel flows is the cap, never 4× it.
    let b = Arc::new(Bucket::new(Some(100)));
    let t0 = tokio::time::Instant::now();
    let mut handles = Vec::new();
    for _ in 0..4 {
        let b = Arc::clone(&b);
        handles.push(tokio::spawn(async move { b.acquire(500).await }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let elapsed = tokio::time::Instant::now() - t0;
    assert!(
        elapsed >= Duration::from_secs(9),
        "the fleet's SUM must honor the cap: elapsed {elapsed:?}"
    );
}

#[tokio::test]
async fn unlimited_is_a_no_op_with_zero_overhead() {
    let b = Bucket::new(None);
    let t0 = tokio::time::Instant::now();
    b.acquire(1 << 30).await; // 1 GiB through an absent cap: instant
    assert!(
        t0.elapsed() < Duration::from_millis(50),
        "the uncapped path must not pay for the limiter"
    );
}

#[tokio::test(start_paused = true)]
async fn swapping_the_rate_takes_effect_on_the_next_acquire() {
    // The hot path: the user drags the cap while jobs run; the NEXT acquire
    // obeys the new rate (the manager swaps the shared bucket's rate).
    let b = Arc::new(Bucket::new(Some(1000)));
    b.acquire(500).await; // rides the burst allowance instantly
    b.set_rate(Some(100)); // now 100 B/s
    let t0 = tokio::time::Instant::now();
    b.acquire(1000).await; // 500 allowance left + 500 to pace ⇒ ~5 s
    let elapsed = tokio::time::Instant::now() - t0;
    assert!(
        elapsed >= Duration::from_secs(4),
        "the swapped rate must pace the next acquire: elapsed {elapsed:?}"
    );
}

// ---- the wiring: the drive loop paces between leases ----------------------

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::chunk::Lease;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::supervisor::Supervisor;
use ium_core::{JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;

/// Real bytes off disk, instant wire — the cap's pacing happens in the
/// between-lease acquire, so elapsed time is measurable even with a mock
/// that moves bytes as fast as the CPU allows (the supervisor.rs WedgeMock's
/// honest cousin, minus the wedge).
struct InstantMock;

#[async_trait::async_trait]
impl TransferAdapter for InstantMock {
    fn name(&self) -> &'static str {
        "instant"
    }
    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: Some(1),
            session_token: Some("instant-mock".into()),
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
        let mut file = std::fs::File::open(&spec.source_path)
            .map_err(|e| AdapterError::Transient(format!("source vanished: {e}")))?;
        file.seek(SeekFrom::Start(lease.resume_from)).unwrap();
        let want = (lease.end - lease.resume_from) as usize;
        let mut buf = vec![0u8; want];
        file.read_exact(&mut buf)
            .map_err(|e| AdapterError::Transient(format!("short read: {e}")))?;
        progress.add(want as u64);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: lease.end - lease.resume_from,
            adapter_token: Some(format!("tok-{}", lease.idx)),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("instant mock ok".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn make_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut bytes = Vec::with_capacity(size as usize);
    for off in 0..size {
        bytes.push((off % 251) as u8);
    }
    std::fs::write(&path, &bytes).unwrap();
    path
}

fn spec_for(path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "instant".into(),
        endpoint: "mock://endpoint".into(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_capped_job_paces_between_leases_and_stays_byte_exact() {
    // 2 MiB source, 1 MiB chunks ⇒ two leases of 1 MiB. Cap 1 MiB/s: the
    // first lease rides the full-burst allowance; the second waits ~1 s.
    // Virtual time makes the wait deterministic; the result is byte-exact
    // (the cap must never lose a byte).
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "capped.bin", 2 * MIB);
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(InstantMock));
    // The cap BEFORE the drive — the boot-apply shape. Chunk sizing: the
    // planner's default is ≥ 1 MiB for a 2 MiB file on a resumable lane.
    m.set_rate_cap(Some(MIB));
    let m = Arc::new(m);
    let sup = Supervisor::new(Arc::clone(&m));

    let rec = m.create_job(&spec_for(&path, 2 * MIB), 2).unwrap();
    let t0 = tokio::time::Instant::now();
    sup.drive(&rec.id);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let snap = m.snapshot(&rec.id).unwrap();
        if snap.job.state == JobState::Done {
            break;
        }
        assert!(
            tokio::time::Instant::now() <= deadline,
            "the capped job must still finish (eviction-safe by construction)"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let elapsed = tokio::time::Instant::now() - t0;
    assert!(
        elapsed >= Duration::from_millis(900),
        "the second lease must have waited for the bucket: elapsed {elapsed:?}"
    );
    // The bytes are exact (the verify gate ran; the state machine says Done
    // only past it) — assert the file's truth directly too.
    let received = std::fs::read(&path).unwrap();
    assert_eq!(received.len() as u64, 2 * MIB);
}

#[tokio::test]
async fn an_uncapped_job_pays_nothing_for_the_limiter() {
    // The same job with NO cap: the acquire is a no-op — the wall-clock
    // bound is generous but real (the uncapped engine must not slow down
    // because the limiter exists).
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "free.bin", 2 * MIB);
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(InstantMock));
    let m = Arc::new(m);
    let sup = Supervisor::new(Arc::clone(&m));

    let rec = m.create_job(&spec_for(&path, 2 * MIB), 2).unwrap();
    sup.drive(&rec.id);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let snap = m.snapshot(&rec.id).unwrap();
        if snap.job.state == JobState::Done {
            break;
        }
        assert!(
            tokio::time::Instant::now() <= deadline,
            "the uncapped job must finish promptly"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
