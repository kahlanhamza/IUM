//! The TUS kill-9 drill child (issue #11) — the s3_drill shape applied to
//! TUS byte-level resume. One binary, two modes, driven by the env-gated
//! parent test `tests/tus_drill.rs`:
//!
//! * `upload --journal J --file F --hold-ms N` — create a real TUS job
//!   through the real manager (2 chunks × SIZE/2) and drive it. Only chunk 0
//!   may reach the server; later arrivals wedge at entry (nothing moves,
//!   nothing confirms — the MockAdapter entry-log discipline). When the
//!   server ACKs chunk 0, the wrapper prints `SERVER_ACK <offset>` and then
//!   HOLDS the confirmation for N ms before returning it to the manager.
//!   The parent SIGKILLs inside that hold — the exact
//!   kill-after-server-ack-before-journal-write window the issue names: the
//!   server holds chunk 0's bytes, the journal holds none of them.
//!   Deliberately stateless beyond the journal — that is the point.
//!
//! * `resume --journal J` — a FRESH process reopens the same SQLite WAL
//!   journal and drives the recovered job with a FRESH adapter whose session
//!   map is EMPTY (no explicit restore — recovering the journaled creation
//!   URL through the manager's `Action::Reattach`, issue #12, is part of
//!   what this drill proves). The first PATCH map-misses into `SessionLost`,
//!   reattach replays the token, and the re-lease walks straight into the
//!   409 HEAD-realign absorption (#10) because the server is a full chunk
//!   ahead of the journal. The wrapper additionally orders chunk 1 after
//!   chunk 0 — TUS is ONE sequential-offset resource; chunk 0 must realign
//!   while the server offset still sits inside its span. Prints
//!   `SERVER_OFFSET <n>` (pre-resume server truth, HEADed from the journaled
//!   Location), `WIRE <bytes>` (what this process pushed) and `DONE <state>`
//!   so the parent can assert the byte-level zero-re-upload guarantee and
//!   the read-only-HEAD verify-before-Done contract.
//!
//! Env: IUM_TUS_ENDPOINT as everywhere else.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ium_core::adapter::{
    AdapterError, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter, VerifyReport,
};
use ium_core::{JobManager, JobSpec, JobState, Lease, SessionJournal, TusAdapter};

/// Gate + telemetry wrapper around [`TusAdapter`].
///
/// * `gate_idx` (upload mode) — only this chunk index may pass; every other
///   call waits (loudly failing after a generous deadline — the parent kills
///   the process long before it fires).
/// * `hold` (upload mode) — after the gated chunk is server-ACKed, print
///   `SERVER_ACK` and keep the confirmation out of the journal for N ms.
///   The parent kills inside this window; server truth and journal truth
///   deliberately diverge for its duration.
/// * `order_after` (resume mode) — calls for indexes above the first wait
///   until that index has completed once (Ok). One sequential-offset
///   resource: no concurrent PATCH, no realign-behind-a-later-chunk chaos.
/// * WIRE — what the progress telemetry reported on every call that ended
///   Ok; failed calls add nothing.
struct DrillAdapter {
    inner: TusAdapter,
    wire: Arc<AtomicU64>,
    gate_idx: Option<u32>,
    hold: Option<(u32, u64, Arc<AtomicBool>)>,
    order_after: Option<(u32, Arc<AtomicBool>)>,
}

impl DrillAdapter {
    fn upload_mode(
        inner: TusAdapter,
        wire: Arc<AtomicU64>,
        hold_ms: u64,
    ) -> (Arc<Self>, Arc<AtomicBool>) {
        let acked = Arc::new(AtomicBool::new(false));
        (
            Arc::new(Self {
                inner,
                wire,
                gate_idx: Some(0),
                hold: Some((0, hold_ms, Arc::clone(&acked))),
                order_after: None,
            }),
            acked,
        )
    }

    fn resume_mode(inner: TusAdapter, wire: Arc<AtomicU64>) -> (Arc<Self>, Arc<AtomicBool>) {
        let first_done = Arc::new(AtomicBool::new(false));
        (
            Arc::new(Self {
                inner,
                wire,
                gate_idx: None,
                hold: None,
                order_after: Some((0, Arc::clone(&first_done))),
            }),
            first_done,
        )
    }

    /// Sleeping wait (not `pending!()`): a wedged drill fails loudly instead
    /// of hanging the nightly forever.
    async fn wait_for(&self, allowed: &AtomicBool, what: &str) -> Result<(), AdapterError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        while !allowed.load(Ordering::SeqCst) {
            if tokio::time::Instant::now() >= deadline {
                return Err(AdapterError::Unknown(format!(
                    "drill gate expired waiting for {what}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl TransferAdapter for DrillAdapter {
    fn name(&self) -> &'static str {
        "tus_drill"
    }

    fn capabilities(&self) -> ium_core::Capability {
        self.inner.capabilities()
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.inner.probe(spec).await
    }

    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.inner.reattach(endpoint, token).await
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        if let Some(gate) = self.gate_idx {
            // Upload mode: only the gated chunk passes; the rest wait (the
            // parent SIGKILLs the whole fleet mid-hold, long before the 120 s
            // deadline turns the wait into a loud failure).
            if lease.idx != gate {
                let never = AtomicBool::new(false);
                self.wait_for(&never, "the upload-mode gate").await?;
            }
        }
        if let Some((first_idx, first_done)) = &self.order_after {
            if lease.idx > *first_idx {
                self.wait_for(first_done, "the ordered first chunk").await?;
            }
        }
        let confirm = self.inner.upload_chunk(spec, lease, progress).await?;
        self.wire.fetch_add(progress.moved(), Ordering::SeqCst);
        if let Some((first_idx, first_done)) = &self.order_after {
            if lease.idx == *first_idx {
                first_done.store(true, Ordering::SeqCst);
            }
        }
        if let Some((ack_idx, hold_ms, acked_once)) = &self.hold {
            // The kill window: server acked, journal has NOT. Print the ack,
            // then sit on the confirm — the manager cannot journal what it
            // has not been handed.
            if lease.idx == *ack_idx && !acked_once.swap(true, Ordering::SeqCst) {
                println!(
                    "SERVER_ACK {}",
                    confirm.adapter_token.clone().unwrap_or_default()
                );
                tokio::time::sleep(Duration::from_millis(*hold_ms)).await;
            }
        }
        Ok(confirm)
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        self.inner.verify(spec).await
    }
}

fn arg_of(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn endpoint() -> String {
    std::env::var("IUM_TUS_ENDPOINT").expect("IUM_TUS_ENDPOINT must be set (harness provides it)")
}

/// Journal-truth progress: the only thing a SIGKILL cannot take away.
fn spawn_watcher(mgr: Arc<JobManager>, id: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Ok(snap) = mgr.snapshot(&id) {
                let confirmed: u64 = snap.chunks.iter().map(|c| c.bytes_confirmed).sum();
                println!("CONFIRMED {confirmed}");
            }
        }
    })
}

async fn run_upload(journal_path: &str, file: &str, hold_ms: u64) -> Result<(), String> {
    let source = std::fs::metadata(file)
        .map_err(|e| format!("source: {e}"))?
        .len();
    let spec = JobSpec {
        source_path: file.to_string(),
        source_size: source,
        adapter: "tus_drill".into(),
        endpoint: endpoint(),
    };

    let journal = SessionJournal::open(journal_path).map_err(|e| e.to_string())?;
    let mut mgr = JobManager::with_journal(journal).map_err(|e| e.to_string())?;
    let wire = Arc::new(AtomicU64::new(0));
    let (holder, _acked) = DrillAdapter::upload_mode(TusAdapter::new(), Arc::clone(&wire), hold_ms);
    mgr.register_adapter(holder);
    let mgr = Arc::new(mgr);

    let rec = mgr
        .create_job(&spec, 2)
        .map_err(|e| format!("create_job: {e}"))?;
    println!("JOB {}", rec.id);

    let watcher = spawn_watcher(Arc::clone(&mgr), rec.id.clone());

    let done = mgr
        .run_job(&rec.id)
        .await
        .map_err(|e| format!("run_job: {e}"))?;
    watcher.abort();
    println!("STATE {}", done.state.as_str());
    Err("upload child finished before the parent killed it — drill degenerate".to_string())
}

async fn run_resume(journal_path: &str) -> Result<(), String> {
    let journal = SessionJournal::open(journal_path).map_err(|e| e.to_string())?;
    let mut mgr = JobManager::with_journal(journal).map_err(|e| e.to_string())?;
    let wire = Arc::new(AtomicU64::new(0));
    // FRESH adapter, EMPTY session map. No explicit restore: the map-miss →
    // SessionLost → manager Action::Reattach (#12) → journaled-token replay
    // path IS part of what this drill proves.
    let inner = TusAdapter::new();

    // Crash recovery: the job sits at `Running`; find it in the journal.
    let jobs = mgr.list_jobs().map_err(|e| e.to_string())?;
    let rec = jobs
        .iter()
        .find(|j| !j.state.is_terminal())
        .ok_or_else(|| "no non-terminal job in the journal".to_string())?
        .clone();

    // Pre-resume server truth, straight from the journaled creation URL.
    let snap = mgr.snapshot(&rec.id).map_err(|e| e.to_string())?;
    let (_, _, payload) = snap
        .events
        .iter()
        .find(|(_, k, _)| k == "probe")
        .ok_or_else(|| "no probe event in the journal".to_string())?;
    let probe: serde_json::Value =
        serde_json::from_str(payload).map_err(|e| format!("probe payload: {e}"))?;
    let location = probe["session_token"]
        .as_str()
        .ok_or_else(|| "probe event has no session_token".to_string())?;
    let client = reqwest::Client::new();
    let resp = client
        .head(location)
        .header("Tus-Resumable", "1.0.0")
        .send()
        .await
        .map_err(|e| format!("server-offset HEAD: {e}"))?;
    let server_offset: u64 = resp
        .headers()
        .get("upload-offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "no Upload-Offset header on the server-offset HEAD".to_string())?;
    println!("SERVER_OFFSET {server_offset}");

    let (adapter, _first_done) = DrillAdapter::resume_mode(inner, Arc::clone(&wire));
    mgr.register_adapter(adapter);
    let mgr = Arc::new(mgr);
    let done = mgr
        .run_job(&rec.id)
        .await
        .map_err(|e| format!("run_job: {e}"))?;
    println!("WIRE {}", wire.load(Ordering::SeqCst));
    println!("DONE {}", done.state.as_str());
    if done.state != JobState::Done {
        return Err(format!(
            "job ended at {} — honest failure, drill fails",
            done.state.as_str()
        ));
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    let journal = arg_of(&args, "--journal").expect("--journal PATH required");
    let result = match mode {
        "upload" => {
            let file = arg_of(&args, "--file").expect("--file PATH required");
            let hold_ms = arg_of(&args, "--hold-ms")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8_000);
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(run_upload(&journal, &file, hold_ms))
        }
        "resume" => tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(run_resume(&journal)),
        other => Err(format!("unknown mode '{other}' (upload|resume)")),
    };
    if let Err(e) = result {
        eprintln!("tus_drill_child: {e}");
        std::process::exit(1);
    }
}
