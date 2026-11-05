//! Lone-worker eviction contract (issue #25): when the impaired connection
//! is the ONLY one left active — every peer finished sub-window and stopped
//! voting — the eviction floor must still see the fleet's achieved rate and
//! revoke the laggard. At HEAD the floor computes over live samples only:
//! fast peers never vote (sub-window completions) and finished leases drop
//! their entry, so the laggard is the lone sample and `below_floor` refuses
//! to judge — the wait-out runs to completion. The #22 real-world S3 drill
//! hit exactly this at 32 KiB/s: the victim part crawled ~472 s with zero
//! `chunk_evicted`.
//!
//! Test shrink (documented, same honesty as #18): the window is 100 ms
//! instead of the production 10 s; ratios (0.4×median) and strikes (2) are
//! the spec defaults. The impaired attempt crawls at ~200 KiB/s-equivalent
//! and would take ~3 s to finish its span alone ("wait-out"); the fleet's
//! fast chunk confirms in well under one window — precisely the shape that
//! starves the floor at HEAD.

#![allow(dead_code)] // test-support helpers shared across fixtures

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::eviction::EvictionPolicy;
use ium_core::{JobManager, JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;
/// Impaired pace: 1 KiB every 5 ms ≈ 200 KiB/s — a real bad-link rate.
const CRAWL_TICK_BYTES: u64 = 1024;
const CRAWL_TICK_MS: u64 = 5;
/// Wait-out bound: at ~200 KiB/s the 8 MiB span would need ~40 s; this test
/// only needs the impaired attempt to OUTLAST the eviction windows (2 ×
/// 100 ms) by a wide margin, then finish honestly if eviction never fires
/// (the HEAD shape) so the RED run fails on the missing event, not a hang.
const CRAWL_MAX_TICKS: u64 = 600; // 600 × 5 ms = 3 s

fn make_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    let mut remaining = size;
    let mut seed = 0x9E3779B97F4A7C15u64;
    while remaining > 0 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let chunk = seed.to_le_bytes();
        let n = remaining.min(8) as usize;
        f.write_all(&chunk[..n]).unwrap();
        remaining -= n as u64;
    }
    path
}

/// Fleet-shaped mock: chunk 0 confirms immediately (sub-window — at HEAD it
/// never casts a vote); chunk 1's FIRST upload crawls until either the
/// manager evicts it (the future is dropped mid-crawl) or the wait-out
/// deadline passes; any later call is the "healed" retry and confirms
/// immediately.
struct LoneMock {
    idx1_calls: AtomicU32,
}

impl LoneMock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            idx1_calls: AtomicU32::new(0),
        })
    }
}

#[async_trait::async_trait]
impl TransferAdapter for LoneMock {
    fn name(&self) -> &'static str {
        "lone_mock"
    }
    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            resumable: true,
            session_token: Some("tok-lone".into()),
            concatenation: false,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
            uniform_block: None,
        })
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        lease: &ium_core::Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let span = lease.end - lease.begin;
        if lease.idx == 0 {
            progress.add(span);
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: span,
                adapter_token: Some("N:etag-0".into()),
            });
        }
        let call = self.idx1_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call > 1 {
            // The healed retry: a fresh connection at full speed.
            progress.add(span);
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: span,
                adapter_token: Some("N:etag-1-retry".into()),
            });
        }
        // First attempt: the impaired link. Crawl until evicted (this
        // future gets dropped) or the wait-out deadline completes.
        for _ in 0..CRAWL_MAX_TICKS {
            tokio::time::sleep(Duration::from_millis(CRAWL_TICK_MS)).await;
            progress.add(CRAWL_TICK_BYTES);
        }
        progress.add(span);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: Some("N:etag-1-waitout".into()),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: None,
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn event_kinds_and_idx(m: &JobManager, job: &str) -> Vec<(String, u64)> {
    m.snapshot(job)
        .unwrap()
        .events
        .into_iter()
        .map(|(_, k, p)| {
            let idx = serde_json::from_str::<serde_json::Value>(&p)
                .ok()
                .and_then(|v| v.get("idx").and_then(|i| i.as_u64()))
                .unwrap_or(u64::MAX);
            (k, idx)
        })
        .collect()
}

#[tokio::test]
async fn lone_impaired_worker_is_evicted_using_finished_fleet_votes() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_source(dir.path(), "lone.bin", 16 * MIB); // 2 chunks × 8 MiB (CHUNK_MIN is 5 MiB)
    let journal = SessionJournal::open(dir.path().join("lone.db")).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    // Test shrink: 100 ms windows; 0.4×median floor and 2 strikes are the
    // spec defaults. Production runs 10 s windows (realworld_stress bin).
    m.set_eviction_policy(EvictionPolicy {
        window: Duration::from_millis(100),
        ..EvictionPolicy::default()
    });
    m.register_adapter(LoneMock::new());

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: 16 * MIB,
        adapter: "lone_mock".into(),
        endpoint: "mock://lone".into(),
    };
    let rec = m.create_job(&spec, 2).unwrap();
    let t0 = std::time::Instant::now();
    let out = m.run_job(&rec.id).await.unwrap();
    let elapsed = t0.elapsed();

    assert_eq!(
        out.state,
        ium_core::state::JobState::Done,
        "the healed retry must complete the job"
    );
    let events = event_kinds_and_idx(&m, &rec.id);
    let evictions: Vec<(String, u64)> = events
        .iter()
        .filter(|(k, _)| k == "chunk_evicted")
        .cloned()
        .collect();
    assert!(
        !evictions.is_empty(),
        "the impaired lone worker must be evicted from the fleet's finished votes \
         (at HEAD the floor goes blind and the wait-out completes): {events:?}"
    );
    assert!(
        evictions.iter().all(|(_, idx)| *idx == 1),
        "the impaired chunk (idx 1) is the eviction target: {evictions:?}"
    );
    // The eviction must fire long before the 3 s wait-out completes —
    // two 100 ms windows plus scheduling.
    assert!(
        elapsed < Duration::from_secs(2),
        "eviction must beat the wait-out by a wide margin, took {elapsed:?}"
    );
}
