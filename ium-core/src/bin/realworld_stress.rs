//! The real-world stress bin (issue #22). Scenarios over the REAL
//! `S3MultipartAdapter` (real SigV4) against the REAL pinned SeaweedFS,
//! driven by `scripts/realworld-stress.sh` under `s3-test.sh
//! --with-realworld`. Every scenario prints ONE machine-checkable line:
//!
//! ```text
//! EVIDENCE {json}
//! ```
//!
//! Scenarios:
//! * `s1` — the plain real upload: 308 MiB film, 4 workers, run to Done;
//!   evidence = elapsed, mean throughput, chunk count.
//! * `s3hostile` — same upload THROUGH the throttle proxy (scripts/
//!   throttle_proxy.py): one link choked to 96 KiB/s for 45 s; production
//!   eviction defaults must fire `chunk_evicted` on live wire numbers;
//!   evidence carries the raw payloads.
//! * `s4pause` — user actions, part 1: drive → frontier ≥ 10% → pause →
//!   state Paused, frontier provably frozen for 2.5 s → resume → Done.
//! * `s4cancel` — user actions, part 2: drive → ≥ 1 part confirmed →
//!   cancel → `Canceled` journaled (the driver then proves the server never
//!   completed the object).
//! * `s5` — concurrent fleet: three real films racing through ONE manager;
//!   all Done; per-job and fleet throughput.
//! * `s6worse` — the worse-case ladder leg (issue #28, owner reality 2026-11-03:
//!   the uplink is B/s-class): the WHOLE uplink is uniformly slow through the
//!   proxy (`--victim-part all`), fleet 1 so the plan is ONE large part the
//!   5 MiB floor regime can actually split. TIMEBOXED — at 512 B/s a 30 MB
//!   film takes ~17 hours, and the leg proves the ENGINE's adaptation, not
//!   the wire's patience. At the box the job is canceled (a cancel under a
//!   dying link must never complete the object). The resplit policy is armed
//!   on EVERY connection — fast links included; it fires here because the
//!   measured median says a part is > 90 s of wire time, the same arithmetic
//!   that declines to churn a fast link.
//! * `s7net` — the real-network models (issue #32): impairment beyond
//!   bandwidth, the application-observable of each. `--model loss` = RTO-scale
//!   stalls on a paced lane (completion + byte-exactness is the assertion;
//!   evictions are recorded honestly — a stalling lease MAY be correctly
//!   struck). `--model rtt` = 250 ms ± jitter RTT, no throttle (an
//!   alive-but-latent link must NEVER be churned: zero evictions, zero
//!   resplits). `--model reset` = a real mid-body RST (SO_LINGER=0 both ends)
//!   must land as ONE counted `chunk_error {kind: Transient, action: Retry}`
//!   and never a Failed. Server byte-exactness is the harness's read-back
//!   job; wire truth is the proxy's own log (LOSS/RESET-WIRE lines).
//!
//! Env: IUM_S3_ENDPOINT / IUM_S3_ACCESS_KEY / IUM_S3_SECRET_KEY /
//! IUM_S3_REGION (see `S3MultipartConfig::from_env`). Eviction policy is the
//! PRODUCTION default everywhere (10 s window, 0.4× median floor, 2 strikes)
//! — no test shrink: the real-world drill earns real-world timings.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use ium_core::chunk::{Lease, CHUNK_MAX, CHUNK_MIN};
use ium_core::eviction::{EvictionPolicy, ResplitPolicy};
use ium_core::state::JobState;
use ium_core::{ChunkState, JobManager, JobSpec, SessionJournal, Supervisor};

fn arg_of(args: &[String], k: &str) -> Option<String> {
    args.iter()
        .position(|a| a == k)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn args_multi(args: &[String], k: &str) -> Vec<String> {
    args.iter()
        .zip(args.iter().skip(1))
        .filter(|(a, _)| a.as_str() == k)
        .map(|(_, v)| v.clone())
        .collect()
}

fn evidence(v: serde_json::Value) {
    println!("EVIDENCE {v}");
}

fn spec_for(file: &str, endpoint: &str) -> Result<JobSpec, String> {
    let size = std::fs::metadata(file)
        .map_err(|e| format!("stat {file}: {e}"))?
        .len();
    Ok(JobSpec {
        source_path: file.to_string(),
        source_size: size,
        adapter: "s3_multipart".into(),
        endpoint: endpoint.to_string(),
    })
}

/// A fresh manager over a fresh journal, with the real adapter from env and
/// PRODUCTION eviction defaults stated loudly.
fn fresh_manager(journal_dir: &str) -> Result<Arc<JobManager>, String> {
    let db = std::path::Path::new(journal_dir).join("journal.db");
    let journal = SessionJournal::open(&db).map_err(|e| format!("journal open: {e}"))?;
    let mut m = JobManager::with_journal(journal).map_err(|e| format!("manager: {e}"))?;
    m.register_adapter(Arc::new(
        S3MultipartAdapter::new(S3MultipartConfig::from_env().map_err(|e| e.to_string())?)
            .map_err(|e| format!("adapter: {e}"))?,
    ));
    m.set_eviction_policy(EvictionPolicy::default());
    Ok(Arc::new(m))
}

/// The S2 wedge technique, in-process (issue #29): the first `wedge_after`
/// upload calls pass through to the real adapter; every later call awaits
/// `pending()` AT ENTRY — zero bytes on the wire, the journal frozen at one
/// confirmed part forever. The cancel window is then GUARANTEED, not raced:
/// on a GitHub runner the loopback server uploads the whole 30 MB film in
/// ~0.2 s and the job reached Done between two 25 ms polls (RED: cloud run
/// 41561114324, "job reached Done before any cancelable moment"). The wedged
/// future is droppable — the cancel tears it off the select! and the job
/// lands Canceled; the multipart is never completed, so the server truth
/// stays HEAD 404.
struct WedgeAdapter {
    inner: Arc<dyn TransferAdapter>,
    /// Calls numbered `wedge_after .. wedge_until` wedge at entry; everything
    /// else passes. `wedge_until = None` wedges every later call forever
    /// (s4cancel — never resumes).
    wedge_after: u32,
    wedge_until: Option<u32>,
    /// How long a wedged call holds before proceeding with the real upload.
    /// `None` = hold forever (s4cancel). A bounded hold gives s4pause a
    /// deterministic plateau: the frontier sits at 25% for the whole hold,
    /// the pause poll cannot miss it, and the release creates the lease
    /// boundary the cooperative Paused needs to journal (cloud RED
    /// 41575158721: the whole film confirmed between two 25 ms polls —
    /// twice; even a 2 MiB/s leg throttle left a sub-second window).
    hold: Option<Duration>,
    calls: AtomicU32,
}

#[async_trait]
impl TransferAdapter for WedgeAdapter {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn capabilities(&self) -> Capability {
        self.inner.capabilities()
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.inner.probe(spec).await
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        let wedged = n >= self.wedge_after && self.wedge_until.map_or(true, |until| n < until);
        if wedged {
            // Wedge at ENTRY: zero wire bytes. The drive loop's select!
            // abandons the unbounded wedge the moment cancel_rx fires; the
            // bounded wedge releases itself after `hold` and proceeds — the
            // cooperative pause needs that lease boundary to journal.
            match self.hold {
                None => {
                    return std::future::pending::<Result<ChunkConfirm, AdapterError>>().await;
                }
                Some(hold) => {
                    let _ = tokio::time::timeout(hold, std::future::pending::<()>()).await;
                }
            }
        }
        self.inner.upload_chunk(spec, lease, progress).await
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        self.inner.verify(spec).await
    }

    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.inner.reattach(endpoint, token).await
    }
}

/// Issue #28 diagnostics: the runner's `S6[2048]` legs refuse to fire the
/// resplit even though every paper condition holds — this wrapper samples
/// the adapter's OWN progress meter every 2 s to stderr, so a dispatched run
/// shows the engine's actual progress curve (burst size, freeze point,
/// per-attempt shape) inside the run log.
struct TelemetryAdapter {
    inner: Arc<dyn TransferAdapter>,
}

#[async_trait]
impl TransferAdapter for TelemetryAdapter {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn capabilities(&self) -> Capability {
        self.inner.capabilities()
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.inner.probe(spec).await
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let sampler = {
            let progress = progress.clone();
            let idx = lease.idx;
            tokio::spawn(async move {
                let t0 = std::time::Instant::now();
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    eprintln!(
                        "[s6worse][progress] idx={idx} t=+{:.1}s moved={}",
                        t0.elapsed().as_secs_f64(),
                        progress.moved()
                    );
                }
            })
        };
        let out = self.inner.upload_chunk(spec, lease, progress).await;
        sampler.abort();
        out
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        self.inner.verify(spec).await
    }

    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.inner.reattach(endpoint, token).await
    }
}

/// A fresh manager whose real adapter reports its progress curve to stderr.
fn telemetry_manager(journal_dir: &str) -> Result<Arc<JobManager>, String> {
    let db = std::path::Path::new(journal_dir).join("journal.db");
    let journal = SessionJournal::open(&db).map_err(|e| format!("journal open: {e}"))?;
    let mut m = JobManager::with_journal(journal).map_err(|e| format!("manager: {e}"))?;
    let real = S3MultipartAdapter::new(S3MultipartConfig::from_env().map_err(|e| e.to_string())?)
        .map_err(|e| format!("adapter: {e}"))?;
    m.register_adapter(Arc::new(TelemetryAdapter {
        inner: Arc::new(real),
    }));
    m.set_eviction_policy(EvictionPolicy::default());
    Ok(Arc::new(m))
}

/// A fresh manager whose real adapter is wrapped in the deterministic wedge.
fn wedge_manager(
    journal_dir: &str,
    wedge_after: u32,
    wedge_until: Option<u32>,
    hold: Option<Duration>,
) -> Result<Arc<JobManager>, String> {
    let db = std::path::Path::new(journal_dir).join("journal.db");
    let journal = SessionJournal::open(&db).map_err(|e| format!("journal open: {e}"))?;
    let mut m = JobManager::with_journal(journal).map_err(|e| format!("manager: {e}"))?;
    let real = S3MultipartAdapter::new(S3MultipartConfig::from_env().map_err(|e| e.to_string())?)
        .map_err(|e| format!("adapter: {e}"))?;
    m.register_adapter(Arc::new(WedgeAdapter {
        inner: Arc::new(real),
        wedge_after,
        wedge_until,
        hold,
        calls: AtomicU32::new(0),
    }));
    m.set_eviction_policy(EvictionPolicy::default());
    Ok(Arc::new(m))
}

fn frontier_of(m: &JobManager, job: &str) -> Result<u64, String> {
    let snap = m.snapshot(job).map_err(|e| e.to_string())?;
    Ok(snap.chunks.iter().map(|c| c.bytes_confirmed).sum())
}

async fn wait_terminal(
    m: &Arc<JobManager>,
    sup: &Supervisor,
    id: &str,
    deadline: Duration,
) -> Result<JobState, String> {
    let t0 = Instant::now();
    loop {
        let rec = m.snapshot(id).map_err(|e| e.to_string())?.job;
        match rec.state {
            JobState::Done | JobState::Failed | JobState::Canceled => return Ok(rec.state),
            _ => {}
        }
        if !sup.is_driving(id) && t0.elapsed() > Duration::from_secs(3) {
            // drive task gone and still non-terminal after grace: re-drive
            // (resume_signal semantics) unless the state says otherwise
            sup.drive(id);
        }
        if t0.elapsed() > deadline {
            return Err(format!("timeout waiting terminal (last {:?})", rec.state));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn s1(journal: &str, file: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    let m = fresh_manager(journal)?;
    let spec = spec_for(file, endpoint)?;
    let size = spec.source_size;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    let done = m.run_job(&rec.id).await.map_err(|e| e.to_string())?;
    let elapsed = t0.elapsed();
    if done.state != JobState::Done {
        return Err(format!("s1 ended {:?} (must be Done)", done.state));
    }
    let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "scenario": "s1", "state": "Done", "size": size,
        "elapsed_s": elapsed.as_secs_f64(),
        "mean_throughput_Bps": size as f64 / elapsed.as_secs_f64(),
        "chunks": snap.chunks.len(),
        "events": snap.events.len(),
    }))
}

async fn s3hostile(journal: &str, file: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    let m = fresh_manager(journal)?;
    let spec = spec_for(file, endpoint)?;
    let size = spec.source_size;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    let done = m.run_job(&rec.id).await.map_err(|e| e.to_string())?;
    let elapsed = t0.elapsed();
    if done.state != JobState::Done {
        return Err(format!("s3hostile ended {:?} (must be Done)", done.state));
    }
    let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
    let evictions: Vec<String> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_evicted")
        .map(|(_, _, p)| p.clone())
        .collect();
    if evictions.is_empty() {
        return Err(
            "s3hostile: no chunk_evicted in the journal — eviction did NOT fire on the \
             impaired link"
                .into(),
        );
    }
    // Issue #28: surface resplits honestly. In VICTIM mode the fast peers
    // outvote the impaired link in the fleet median, so this list is expected
    // to be EMPTY here — the leg proves eviction, not resplit. Hiding the
    // field would be the dishonest version of that fact.
    let resplits: Vec<serde_json::Value> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_resplit")
        .map(|(_, _, p)| {
            serde_json::from_str::<serde_json::Value>(p)
                .expect("chunk_resplit payload must be json")
        })
        .collect();
    Ok(serde_json::json!({
        "scenario": "s3hostile", "state": "Done", "size": size,
        "elapsed_s": elapsed.as_secs_f64(),
        "mean_throughput_Bps": size as f64 / elapsed.as_secs_f64(),
        "evictions": evictions,
        "resplits": resplits,
    }))
}

async fn s4pause(journal: &str, file: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    // Issue #29, second face — TWICE RED on runners (41569827742, then
    // 41575158721 even through a 2 MiB/s leg throttle): the poll cannot
    // catch a sub-second confirm window. The BOUNDED wedge is the
    // deterministic answer: parts 2-4 hold at entry for 10 s — the frontier
    // sits at 25% (>= the 10% pause point) for TEN seconds, the poll cannot
    // miss it, and the release creates the lease boundary the cooperative
    // Paused needs to journal. Zero race, zero engine-policy change.
    let m = wedge_manager(journal, 1, Some(4), Some(Duration::from_secs(10)))?;
    let sup = Supervisor::new(Arc::clone(&m));
    let spec = spec_for(file, endpoint)?;
    let size = spec.source_size;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    sup.drive(&rec.id);

    // pause once ≥10% is confirmed (whole parts are atomic, so the frontier
    // moves in part-sized steps). ONE snapshot is the decision input: state
    // AND frontier from the same journal view — a terminal job must never
    // reach pause(); that outcome is the honest "upload too fast" failure.
    let frontier_at_pause = loop {
        let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
        let f: u64 = snap.chunks.iter().map(|c| c.bytes_confirmed).sum();
        if matches!(
            snap.job.state,
            JobState::Done | JobState::Failed | JobState::Canceled
        ) {
            return Err(format!(
                "s4pause: job reached state {:?} before the 10% pause point \
                        (upload too fast on this link for a mid-flight pause)",
                snap.job.state
            ));
        }
        if f * 10 >= size {
            sup.pause(&rec.id).map_err(|e| e.to_string())?;
            break f;
        }
        if t0.elapsed() > Duration::from_secs(120) {
            return Err("s4pause: timeout waiting for the 10% frontier".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    // Pause is COOPERATIVE ("takes effect between leases" — supervisor.rs):
    // the drive task journals Paused at the next lease boundary, and on a
    // real KB/s uplink one lease can run for minutes. The journaled state is
    // the only authority — wait for it, bounded; never assume it is instant.
    let pause_t0 = Instant::now();
    let (paused_state, frontier_at_paused) = loop {
        let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
        if snap.job.state == JobState::Paused {
            break (snap.job.state, frontier_of(&m, &rec.id)?);
        }
        if pause_t0.elapsed() > Duration::from_secs(120) {
            return Err(format!(
                "s4pause: pause never landed in the journal (state {:?} after 120 s)",
                snap.job.state
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let pause_latency_s = pause_t0.elapsed().as_secs_f64();
    // the wire must be frozen: 2.5 s after the JOURNALED Paused the frontier
    // is byte-identical. (The pre-Paused grace lease landing is expected and
    // recorded — the freeze proof starts at the journaled state.)
    tokio::time::sleep(Duration::from_secs(2) + Duration::from_millis(500)).await;
    let frontier_frozen = frontier_of(&m, &rec.id)?;
    if frontier_frozen != frontier_at_paused {
        return Err(format!(
            "s4pause: frontier moved while Paused ({frontier_at_paused} → {frontier_frozen})"
        ));
    }
    sup.resume_signal(&rec.id).map_err(|e| e.to_string())?;
    let final_state = wait_terminal(&m, &sup, &rec.id, Duration::from_secs(600)).await?;
    if final_state != JobState::Done {
        return Err(format!("s4pause: resumed job ended {final_state:?}"));
    }
    Ok(serde_json::json!({
        "scenario": "s4pause", "paused_state": format!("{paused_state:?}"),
        "frontier_at_pause_call": frontier_at_pause,
        "frontier_at_journaled_paused": frontier_at_paused,
        "pause_latency_s": pause_latency_s,
        "frontier_after_2_5s": frontier_frozen,
        "final_state": "Done",
        "size": size, "total_elapsed_s": t0.elapsed().as_secs_f64(),
    }))
}

async fn s4cancel(journal: &str, file: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    // Issue #29: the cancel window is GUARANTEED by the in-process wedge (the
    // first upload call passes, every later call awaits at entry) — on a CI
    // runner the whole 30 MB film previously completed between two polls and
    // the scenario never saw a cancelable moment. The journal is provably
    // stable at one confirmed part when the cancel lands.
    let m = wedge_manager(journal, 1, None, None)?;
    let sup = Supervisor::new(Arc::clone(&m));
    let spec = spec_for(file, endpoint)?;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    sup.drive(&rec.id);
    // cancel once ≥1 whole part is confirmed. ONE snapshot is the decision
    // input (state AND frontier from the same journal view). With the wedge
    // the frontier cannot run away: later calls never move a byte.
    let frontier_at_cancel = loop {
        let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
        let f: u64 = snap.chunks.iter().map(|c| c.bytes_confirmed).sum();
        if matches!(
            snap.job.state,
            JobState::Done | JobState::Failed | JobState::Unresumable
        ) {
            return Err(format!(
                "s4cancel: job reached {:?} before any cancelable moment \
                        (upload too fast on this link)",
                snap.job.state
            ));
        }
        if f > 0 {
            break f;
        }
        if t0.elapsed() > Duration::from_secs(120) {
            return Err("s4cancel: timeout waiting for the first confirmation".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let canceled = sup.cancel(&rec.id).await.map_err(|e| e.to_string())?;
    if canceled.state != JobState::Canceled {
        return Err(format!(
            "s4cancel: state after cancel is {:?}",
            canceled.state
        ));
    }
    Ok(serde_json::json!({
        "scenario": "s4cancel", "state": "Canceled",
        "frontier_at_cancel": frontier_at_cancel,
        "size": spec.source_size,
        "elapsed_s": t0.elapsed().as_secs_f64(),
    }))
}

/// Issue #28 — the worse-case ladder leg. The WHOLE uplink is B/s-class
/// (uniform impairment through the proxy; the owner's measured reality, not
/// one bad peer). Fleet 1 so the plan is ONE large part the floor regime can
/// actually split (fleet 4 on the 30 MB film plans ~7.5 MiB parts that are
/// unsplittable at the 5 MiB S3 floor — `plan()` refuses, correctly).
///
/// TIMEBOXED, deliberately not run-to-Done: at 512 B/s a 30 MB film takes
/// ~17 hours, and this leg proves the ENGINE's adaptation on a dying-class
/// link, not the wire's patience. At the timebox the job is canceled — a
/// cancel under a dying link must never complete the object.
///
/// Assertions, all from journal truth (the resplit policy is armed on EVERY
/// connection — it fires here because the measured median says a part is
/// more than `slow_confirm` of wire time, the same arithmetic that declines
/// to churn a fast link):
///   1. exactly ONE `chunk_resplit` fired, with a REAL measured median and a
///      self-consistent floor-respecting target:
///      `target == clamp(median * target_secs, CHUNK_MIN, CHUNK_MAX)`;
///   2. every piece still open (Pending/Leased) after the reshape is
///      `>= CHUNK_MIN` — the S3 5 MiB non-final-part protocol minimum;
///   3. the cancel is honored mid-slow-upload: state Canceled.
///
/// Wire truth (bytes actually pumped at the impaired rate) is the harness's
/// job — the proxy's VICTIM-WIRE log is the only honest meter of the wire.
async fn s6worse(
    journal: &str,
    file: &str,
    endpoint: &str,
    rate_bps: u64,
    timebox: Duration,
) -> Result<serde_json::Value, String> {
    let m = telemetry_manager(journal)?;
    let sup = Supervisor::new(Arc::clone(&m));
    let spec = spec_for(file, endpoint)?;
    let rec = m.create_job(&spec, 1).map_err(|e| e.to_string())?; // ONE large part
    let t0 = Instant::now();
    sup.drive(&rec.id);
    tokio::time::sleep(timebox).await;
    let frontier_at_cancel = frontier_of(&m, &rec.id)?;
    let canceled = sup.cancel(&rec.id).await.map_err(|e| e.to_string())?;
    if canceled.state != JobState::Canceled {
        return Err(format!(
            "s6worse: state after cancel is {:?} (must be Canceled)",
            canceled.state
        ));
    }
    let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
    let resplits: Vec<serde_json::Value> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_resplit")
        .map(|(_, _, p)| {
            serde_json::from_str::<serde_json::Value>(p)
                .expect("chunk_resplit payload must be json")
        })
        .collect();
    if resplits.is_empty() {
        // Self-diagnosing RED (issue #28): the cloud run 41568413124 failed
        // this leg with no visibility — carry the journal's shape in the
        // error so the next RED reads its own cause.
        let kinds: std::collections::BTreeMap<&str, usize> =
            snap.events
                .iter()
                .fold(std::collections::BTreeMap::new(), |mut m, (_, k, _)| {
                    *m.entry(k.as_str()).or_insert(0) += 1;
                    m
                });
        let shape: Vec<String> = snap
            .chunks
            .iter()
            .map(|c| {
                format!(
                    "#{} {}..{} {:?} confirmed={}",
                    c.idx, c.begin, c.end, c.state, c.bytes_confirmed
                )
            })
            .collect();
        return Err(format!(
            "s6worse: no chunk_resplit in the journal — the resplit policy never \
             fired on a B/s-class link (issue #28 RED); events={kinds:?} chunks={shape:?}"
        ));
    }
    if resplits.len() > 1 {
        return Err(format!(
            "s6worse: {} resplits in one timebox — hysteresis must hold the \
             plan at one reshape in the floor regime",
            resplits.len()
        ));
    }
    let payload = &resplits[0];
    let median_bps = payload["median_bps"]
        .as_f64()
        .ok_or("s6worse: resplit median missing")?;
    // median >= 0: > 0 is the measured-rate path; exactly 0 is the FROZEN
    // rule (cloud RED 41571828134: the adapter's writes never completed into
    // the runner's tiny buffers — nothing was measurable, and the floor is
    // the only honest target).
    if !(median_bps.is_finite() && median_bps >= 0.0) {
        return Err(format!(
            "s6worse: resplit median is not a real measured rate: {median_bps}"
        ));
    }
    // Self-consistency, not assumption: the target must be exactly what the
    // recorded median implies under the production policy (its 30 s of wire
    // time), floor/ceiling clamped. The median itself is the engine's own
    // measurement — if the socket buffers inflate it, the target follows it,
    // and the proxy's wire log is where the truth is checked, not here.
    let pol = ResplitPolicy::default();
    let want_target = ((median_bps * pol.target_secs) as u64).clamp(CHUNK_MIN, CHUNK_MAX);
    let target = payload["target"]
        .as_u64()
        .ok_or("s6worse: target missing")?;
    if target != want_target {
        return Err(format!(
            "s6worse: resplit target {target} != clamp(median {median_bps:.0} B/s × \
             {:.0} s) = {want_target}",
            pol.target_secs
        ));
    }
    // The floor: every still-open piece must be >= CHUNK_MIN after the
    // reshape — no infinite split below the S3 protocol minimum.
    let open_piece_min = snap
        .chunks
        .iter()
        .filter(|c| matches!(c.state, ChunkState::Pending | ChunkState::Leased))
        .map(|c| c.end - c.begin)
        .min();
    if let Some(min_span) = open_piece_min {
        if min_span < CHUNK_MIN {
            return Err(format!(
                "s6worse: an open piece spans {min_span} B — below the \
                 5 MiB S3 protocol floor"
            ));
        }
    }
    Ok(serde_json::json!({
        "scenario": "s6worse", "state": "Canceled",
        "rate_Bps": rate_bps, "timebox_s": timebox.as_secs_f64(),
        "size": spec.source_size,
        "elapsed_s": t0.elapsed().as_secs_f64(),
        "frontier_at_cancel": frontier_at_cancel,
        "resplit_median_Bps": median_bps,
        "resplit_target": target,
        "resplit_chunks_before": payload["chunks_before"],
        "resplit_chunks_after": payload["chunks_after"],
        "open_piece_min": open_piece_min,
    }))
}

/// Issue #34 — the long-horizon leg. The #28 ladder legs are MINUTES; the
/// owner's link is HOURS (30 MB @ 512 B/s ≈ 17 h). This scenario runs the
/// real S3 adapter through the uniform-throttle proxy for `--seconds`
/// (the TIMEBOX) and then ends the leg HONESTLY:
///
/// * done before the timebox → `Done` (a short leg is still a leg);
/// * timebox fires first → PAUSE, never cancel — the leg must stay
///   RESUMABLE, which is exactly what an overnight run needs when the box
///   must be reclaimed;
///
/// and in BOTH cases: fresh manager over the same journal (the fresh-process
/// replay), resume to `Done`, verify-before-declare intact, byte-exactness
/// proven by the harness's server read-back (the caller's job, as in #32).
/// Persistence telemetry every 5 s: `[longhaul][persist]` lines with the
/// frontier, the journal's on-disk bytes, and the process RSS high-water —
/// the numbers an overnight artifact needs.
async fn longhaul(
    journal: &str,
    file: &str,
    endpoint: &str,
    rate_bps: u64,
    timebox: Duration,
) -> Result<serde_json::Value, String> {
    let m = telemetry_manager(journal)?;
    let sup = Supervisor::new(Arc::clone(&m));
    let spec = spec_for(file, endpoint)?;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();

    // Persistence telemetry: frontier, journal bytes, RSS high-water, and
    // the (t, rss) sample series the plateau pin consumes (R3, issue #95).
    let journal_dir = journal.to_string();
    let rss_hwm = Arc::new(AtomicU64::new(0));
    let samples: Arc<Mutex<Vec<(f64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let fd_span: Arc<Mutex<Option<(usize, usize)>>> = Arc::new(Mutex::new(None));
    let persist = {
        let id = rec.id.clone();
        let m_tel = Arc::clone(&m);
        let rss_hwm = Arc::clone(&rss_hwm);
        let samples = Arc::clone(&samples);
        let fd_span = Arc::clone(&fd_span);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let frontier = frontier_of(&m_tel, &id).unwrap_or(0);
                let jb = journal_bytes(&journal_dir);
                let rss = rss_kb();
                rss_hwm.fetch_max(rss, Ordering::Relaxed);
                if let Ok(mut s) = samples.lock() {
                    s.push((t0.elapsed().as_secs_f64(), rss));
                }
                if let Ok(mut fd) = fd_span.lock() {
                    if let Some(n) = fd_count() {
                        *fd = Some(match *fd {
                            Some((first, _)) => (first, n),
                            None => (n, n),
                        });
                    }
                }
                eprintln!(
                    "[longhaul][persist] t=+{:.1}s frontier={frontier} journal_bytes={jb} rss_kb={rss}",
                    t0.elapsed().as_secs_f64()
                );
            }
        })
    };

    sup.drive(&rec.id);
    tokio::time::sleep(timebox).await;

    // Honest timebox: pause (resumable), never cancel. If the job already
    // finished inside the timebox, the leg is a `Done` leg.
    let mid_state = m.snapshot(&rec.id).map_err(|e| e.to_string())?.job.state;
    let (frontier_at_pause, paused_s) = if matches!(
        mid_state,
        JobState::Done | JobState::Failed | JobState::Canceled
    ) {
        (frontier_of(&m, &rec.id)?, t0.elapsed().as_secs_f64())
    } else {
        let frontier_at_pause = frontier_of(&m, &rec.id)?;
        sup.pause(&rec.id).map_err(|e| e.to_string())?;
        // Pause is COOPERATIVE (supervisor.rs): it lands at the next LEASE
        // boundary — on a paced link one 5 MiB-floor part IS minutes of wire,
        // so the bound must be part-scale, not poll-scale. The overnight
        // operator sizes it via IUM_LONGHAUL_PAUSE_SECS (default 300 s);
        // the persist lines show the frontier frozen at part granularity —
        // S3 parts are atomic mid-flight (bytes_confirmed is server-acked
        // truth, ADR-002).
        let pause_bound = Duration::from_secs(
            std::env::var("IUM_LONGHAUL_PAUSE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
        );
        let pt0 = Instant::now();
        loop {
            let st = m.snapshot(&rec.id).map_err(|e| e.to_string())?.job.state;
            if st == JobState::Paused {
                break;
            }
            if pt0.elapsed() > pause_bound {
                return Err(format!(
                    "longhaul: pause never landed in the journal (state {st:?} after {:.0} s \
                     — a lease boundary is the landing spot; size IUM_LONGHAUL_PAUSE_SECS \
                     past one part's wire time at this rate)",
                    pt0.elapsed().as_secs_f64()
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        (frontier_at_pause, pt0.elapsed().as_secs_f64())
    };
    let mid_state_final = m.snapshot(&rec.id).map_err(|e| e.to_string())?.job.state;
    let journal_bytes_at_pause = journal_bytes(journal);

    // Fresh-process replay: a NEW manager over the SAME journal (the
    // overnight run's "next session"), resume, wait terminal. The resume
    // runs on the SAME paced lane (the honest next-session shape), so the
    // deadline is sized by the operator for the box:
    // IUM_LONGHAUL_RESUME_SECS (default 900).
    let m2 = telemetry_manager(journal)?;
    let sup2 = Supervisor::new(Arc::clone(&m2));
    sup2.drive(&rec.id);
    let resumed_deadline = Duration::from_secs(
        std::env::var("IUM_LONGHAUL_RESUME_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900),
    );
    let final_state = wait_terminal(&m2, &sup2, &rec.id, resumed_deadline).await?;
    if final_state != JobState::Done {
        return Err(format!(
            "longhaul: resumed leg ended {final_state:?} (must be Done — \
             journal replayable and state truthful afterwards)"
        ));
    }
    // Journal truth after the whole arc: the state event trail must be
    // intact (no half-transitions), and verify-before-declare held (Done is
    // only journaled through the adapter's verify).
    let snap = m2.snapshot(&rec.id).map_err(|e| e.to_string())?;
    let state_events = snap.events.iter().filter(|(_, k, _)| k == "state").count();
    if state_events == 0 {
        return Err("longhaul: no state events in the journal after replay".into());
    }
    if !snap.events.iter().any(|(_, k, _)| k == "verified") {
        return Err(
            "longhaul: Done without a `verified` event — verify-before-declare bypassed".into(),
        );
    }
    // The plateau pin (R3, issue #95): the hours-shaped leg must hold FLAT
    // memory and return every descriptor it opened — through the pause AND
    // the fresh-process replay (both are in the sample span). Warmup is
    // dropped (boot-time allocation is not a leak); a slope past the bound
    // or fd drift fails the leg loudly. Short legs skip LOUD, never silently.
    let n_samples = samples.lock().map_err(|e| e.to_string())?.len();
    let plateau = if n_samples >= 40 {
        let slope = {
            let s = samples.lock().map_err(|e| e.to_string())?;
            let cut = s.len() / 4;
            slope_kib_per_hour(&s[cut..])
        };
        if slope > PLATEAU_KIB_PER_HOUR {
            return Err(format!(
                "longhaul: RSS slope {slope:.0} KiB/hour past the {} KiB/hour bound — \
                 the hours-shaped leg LEAKS ({n_samples} samples)",
                PLATEAU_KIB_PER_HOUR as u64
            ));
        }
        serde_json::json!({
            "slope_kib_per_hour": slope,
            "bound_kib_per_hour": PLATEAU_KIB_PER_HOUR,
            "samples": n_samples,
        })
    } else {
        serde_json::json!({
            "skipped": true,
            "reason": format!("samples {n_samples} < 40 — plateau needs 200 s of telemetry"),
            "samples": n_samples,
        })
    };
    let fd_report = {
        let fd = fd_span.lock().map_err(|e| e.to_string())?;
        match *fd {
            Some((first, last)) => {
                let delta = last.saturating_sub(first);
                if delta > BOUND_FD_DELTA {
                    return Err(format!(
                        "longhaul: fd drift {delta} over the leg (first {first}, last {last}) — \
                         handles held across pause+replay"
                    ));
                }
                serde_json::json!({ "first": first, "last": last, "delta": delta, "bound": BOUND_FD_DELTA })
            }
            None => serde_json::json!({ "skipped": "no /proc/self/fd (non-Linux)" }),
        }
    };
    eprintln!("[longhaul][plateau] {plateau} fd_span {fd_report}");
    persist.abort();
    Ok(serde_json::json!({
        "scenario": "longhaul", "state": "Done",
        "rate_Bps": rate_bps, "timebox_s": timebox.as_secs_f64(),
        "size": spec.source_size,
        "elapsed_s": t0.elapsed().as_secs_f64(),
        "frontier_at_pause": frontier_at_pause,
        "pause_landed_s": paused_s,
        "mid_state": mid_state_final.as_str(),
        "journal_bytes_at_pause": journal_bytes_at_pause,
        "rss_hwm_kb": rss_hwm.load(Ordering::Relaxed),
        "state_events": state_events,
        "replayed_and_resumed": true,
        "plateau": plateau,
        "fd_span": fd_report,
    }))
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
}

/// Open descriptors; `None` off-Linux — the R3 fd pin skips loud there
/// (issue #95, the #34 honesty contract).
fn fd_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").map(|d| d.count()).ok()
}

/// R3 (issue #95): after warmup the leg's RSS may climb at most 24 MiB per
/// hour — generous against allocator noise, tight against real leaks (a
/// 1 KiB-per-tick leak at the 5 s cadence is 720 KiB/hour: caught).
const PLATEAU_KIB_PER_HOUR: f64 = 24.0 * 1024.0;

/// R3 (issue #95): descriptors at the leg's end vs its start — the span
/// covers the pause AND the fresh-process replay.
const BOUND_FD_DELTA: usize = 4;

/// Least-squares slope in KiB per HOUR over (seconds, KiB) samples. A
/// leaking leg climbs; an honest leg is flat or SHRINKS (the allocator
/// returning pages is the opposite of a leak — negative slope passes).
fn slope_kib_per_hour(samples: &[(f64, u64)]) -> f64 {
    let n = samples.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let sx: f64 = samples.iter().map(|(x, _)| x).sum();
    let sy: f64 = samples.iter().map(|(_, y)| *y as f64).sum();
    let sxx: f64 = samples.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = samples.iter().map(|(x, y)| x * (*y as f64)).sum();
    let denom = n * sxx - sx * sx;
    if denom == 0.0 {
        return 0.0;
    }
    (n * sxy - sx * sy) / denom * 3600.0
}

fn journal_bytes(dir: &str) -> u64 {
    ["journal.db", "journal.db-wal", "journal.db-shm"]
        .iter()
        .map(|f| {
            std::fs::metadata(std::path::Path::new(dir).join(f))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum()
}

/// Issue #32 — the real-network models. The proxy (scripts/throttle_proxy.py)
/// now reproduces the APPLICATION-OBSERVABLE of each impairment a userspace
/// relay can produce: loss = RTO-scale stalls with continuation, latency =
/// per-connection connect/response RTT, reset = a real one-shot mid-body RST.
/// This scenario owns the JOURNAL truth per model; the harness owns wire truth
/// (the proxy's LOSS/RESET-WIRE lines) and server truth (read-back sha256).
///
/// * loss — completion IS the assertion: uniform stalls drag the fleet median
///   down with them, so no eviction is expected; if the policy strikes a
///   stalling lease anyway that is CORRECT (a fresh connection escapes the
///   bad patch) and is recorded, never hidden. Byte-exactness + Done +
///   bounded elapsed.
/// * rtt — the sharp one: an alive-but-latent link must NEVER be churned.
///   ZERO `chunk_evicted`, ZERO `chunk_resplit`, Done, bounded elapsed.
/// * reset — the RST must be absorbed as exactly the designed machinery: at
///   least one `chunk_error {kind: Transient, action: Retry}` (manager.rs:
///   send_request error → AdapterError::Transient → counted retry, budget
///   charged once), job Done, never Failed, never a false Done.
async fn s7net(
    journal: &str,
    file: &str,
    endpoint: &str,
    model: &str,
) -> Result<serde_json::Value, String> {
    let deadline = match model {
        "loss" => Duration::from_secs(600),
        "rtt" | "reset" => Duration::from_secs(300),
        other => return Err(format!("s7net: unknown model '{other}' (loss|rtt|reset)")),
    };
    let m = fresh_manager(journal)?;
    let spec = spec_for(file, endpoint)?;
    let size = spec.source_size;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    let done = m.run_job(&rec.id).await.map_err(|e| e.to_string())?;
    let elapsed = t0.elapsed();
    if done.state != JobState::Done {
        return Err(format!(
            "s7net[{model}] ended {:?} (must be Done)",
            done.state
        ));
    }
    if elapsed > deadline {
        return Err(format!(
            "s7net[{model}] took {:.1}s — past the {deadline:?} bound; the model \
             crippled the upload instead of probing it",
            elapsed.as_secs_f64()
        ));
    }
    let snap = m.snapshot(&rec.id).map_err(|e| e.to_string())?;
    let evictions: Vec<String> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_evicted")
        .map(|(_, _, p)| p.clone())
        .collect();
    let resplits: Vec<serde_json::Value> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_resplit")
        .map(|(_, _, p)| {
            serde_json::from_str::<serde_json::Value>(p)
                .expect("chunk_resplit payload must be json")
        })
        .collect();
    let errors: Vec<serde_json::Value> = snap
        .events
        .iter()
        .filter(|(_, k, _)| k == "chunk_error")
        .map(|(_, _, p)| {
            serde_json::from_str::<serde_json::Value>(p).expect("chunk_error payload must be json")
        })
        .collect();
    match model {
        "rtt" => {
            if !evictions.is_empty() {
                return Err(format!(
                    "s7net[rtt]: {} chunk_evicted on an alive-but-latent link — \
                     latency must never churn a healthy fleet (issue #32 RED)",
                    evictions.len()
                ));
            }
            if !resplits.is_empty() {
                return Err(format!(
                    "s7net[rtt]: {} chunk_resplit on a fast lane — 250 ms RTT must \
                     not push any part past the 90 s slow-confirm horizon",
                    resplits.len()
                ));
            }
        }
        "reset" => {
            let retries: Vec<&serde_json::Value> = errors
                .iter()
                .filter(|e| {
                    e["kind"].as_str().is_some_and(|k| k.contains("Transient"))
                        && e["action"].as_str().is_some_and(|a| a.starts_with("Retry"))
                })
                .collect();
            if retries.is_empty() {
                // Self-diagnosing RED: carry the journal's shape so the next
                // RED reads its own cause (the #28 lesson).
                let kinds: std::collections::BTreeMap<&str, usize> = snap.events.iter().fold(
                    std::collections::BTreeMap::new(),
                    |mut m, (_, k, _)| {
                        *m.entry(k.as_str()).or_insert(0) += 1;
                        m
                    },
                );
                return Err(format!(
                    "s7net[reset]: no counted Transient retry in the journal — the \
                     mid-body RST was not absorbed as designed (issue #32 RED); \
                     events={kinds:?} errors={errors:?}"
                ));
            }
        }
        "loss" => {
            // Completion + byte-exactness (harness read-back) IS the
            // assertion; evictions/resplits are recorded honestly above.
        }
        _ => unreachable!("model validated above"),
    }
    Ok(serde_json::json!({
        "scenario": "s7net", "model": model, "state": "Done", "size": size,
        "elapsed_s": elapsed.as_secs_f64(),
        "mean_throughput_Bps": size as f64 / elapsed.as_secs_f64(),
        "evictions": evictions,
        "resplits": resplits,
        "chunk_errors": errors,
    }))
}

async fn s5(
    journal: &str,
    files: &[String],
    bucket_base: &str,
) -> Result<serde_json::Value, String> {
    if files.len() < 2 {
        return Err("s5 needs at least 2 --file entries".into());
    }
    let m = fresh_manager(journal)?;
    let sup = Supervisor::new(Arc::clone(&m));
    let mut ids = Vec::new();
    for f in files {
        let name = std::path::Path::new(f)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("bad file name")?
            .to_string();
        let endpoint = format!("{}/{}", bucket_base.trim_end_matches('/'), name);
        let spec = spec_for(f, &endpoint)?;
        let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
        ids.push((rec.id, name, spec.source_size, Instant::now()));
    }
    for (id, _, _, _) in &ids {
        sup.drive(id);
    }
    let mut jobs = Vec::new();
    for (id, name, size, t0) in &ids {
        let st = wait_terminal(&m, &sup, id, Duration::from_secs(1200)).await?;
        let elapsed = t0.elapsed();
        if st != JobState::Done {
            return Err(format!("s5: job {name} ended {st:?}"));
        }
        jobs.push(serde_json::json!({
            "file": name, "state": "Done", "size": size,
            "elapsed_s": elapsed.as_secs_f64(),
            "mean_throughput_Bps": *size as f64 / elapsed.as_secs_f64(),
        }));
    }
    let fleet_bytes: u64 = ids.iter().map(|(_, _, s, _)| s).sum();
    let fleet_elapsed = ids
        .iter()
        .map(|(_, _, _, t)| t.elapsed().as_secs_f64())
        .fold(0.0_f64, f64::max);
    Ok(serde_json::json!({
        "scenario": "s5", "jobs": jobs,
        "fleet_bytes": fleet_bytes,
        "fleet_wall_s": fleet_elapsed,
        "fleet_throughput_Bps": fleet_bytes as f64 / fleet_elapsed,
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("");
    let journal = arg_of(&args, "--journal").expect("--journal DIR required");
    let result = match scenario {
        "s1" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s1(&journal, &file, &endpoint))
        }
        "s3hostile" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s3hostile(&journal, &file, &endpoint))
        }
        "s4pause" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s4pause(&journal, &file, &endpoint))
        }
        "s4cancel" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s4cancel(&journal, &file, &endpoint))
        }
        "s5" => {
            let files = args_multi(&args, "--file");
            let base = arg_of(&args, "--bucket-base").expect("--bucket-base required");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s5(&journal, &files, &base))
        }
        "s6worse" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            let rate: u64 = arg_of(&args, "--rate")
                .expect("--rate B/s required")
                .parse()
                .expect("--rate must be an integer bytes/s");
            let secs: u64 = arg_of(&args, "--seconds")
                .expect("--seconds required")
                .parse()
                .expect("--seconds must be an integer");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s6worse(
                    &journal,
                    &file,
                    &endpoint,
                    rate,
                    Duration::from_secs(secs),
                ))
        }
        "s7net" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            let model = arg_of(&args, "--model").expect("--model loss|rtt|reset required");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(s7net(&journal, &file, &endpoint, &model))
        }
        "longhaul" => {
            let file = arg_of(&args, "--file").expect("--file required");
            let endpoint = arg_of(&args, "--endpoint").expect("--endpoint required");
            let rate: u64 = arg_of(&args, "--rate")
                .expect("--rate B/s required")
                .parse()
                .expect("--rate must be an integer bytes/s");
            let secs: u64 = arg_of(&args, "--seconds")
                .expect("--seconds required")
                .parse()
                .expect("--seconds must be an integer");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(longhaul(
                    &journal,
                    &file,
                    &endpoint,
                    rate,
                    Duration::from_secs(secs),
                ))
        }
        other => Err(format!(
            "unknown scenario '{other}' (s1|s3hostile|s4pause|s4cancel|s5|s6worse|s7net|longhaul)"
        )),
    };
    match result {
        Ok(v) => evidence(v),
        Err(e) => {
            eprintln!("realworld_stress: {e}");
            std::process::exit(1);
        }
    }
}
