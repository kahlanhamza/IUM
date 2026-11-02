//! The kill-9 drill child (issue #9). One binary, two modes, driven by the
//! env-gated parent test `tests/s3_drill.rs`:
//!
//! * `upload --journal J --file F [--wedge-after N]` — create a real S3 job
//!   through the real manager and drive it. After N *completed* upload calls,
//!   every later call wedges at entry (`pending!()` — the MockAdapter
//!   entry-log discipline: nothing moves, nothing confirms), so the job can
//!   never finish. The parent watches `CONFIRMED` lines from the journal and
//!   SIGKILLs this process mid-body. Deliberately stateless beyond the
//!   journal — that is the point.
//!
//! * `resume --journal J` — a FRESH process reopens the same SQLite WAL
//!   journal, replays the journaled probe event into `restore_session` (the
//!   issue #7 reattach path), counts every span its adapter pushes to the
//!   wire, and drives the recovered job to completion. Prints `WIRE <bytes>`
//!   and `DONE <state>` so the parent can assert the zero-re-upload
//!   guarantee and the ADR-010 verify-before-Done contract.
//!
//! Env: IUM_S3_ENDPOINT / IUM_S3_ACCESS_KEY / IUM_S3_SECRET_KEY /
//! IUM_S3_REGION as everywhere else (see `S3MultipartConfig::from_env`).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use ium_core::adapter::{
    AdapterError, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter, VerifyReport,
};
use ium_core::{
    JobManager, JobSpec, JobState, Lease, S3MultipartAdapter, S3MultipartConfig, SessionJournal,
};

/// Delegating adapter that (a) counts every span pushed onto the wire and
/// (b) can wedge calls at entry — the drill's throttle. The gate is
/// ARRIVAL-based (atomic take-a-ticket), not completion-based: with
/// concurrent workers, calls that arrive before the first call completes
/// would all pass a completed-count check and race the drill into
/// nothing-to-resume. Tickets guarantee exactly `allow_calls` calls ever
/// reach the real adapter; the rest wedge at entry having moved nothing.
struct MeasuringAdapter {
    inner: S3MultipartAdapter,
    wire: Arc<AtomicU64>,
    /// First N arrivals proceed; every later call wedges forever. 0 = all.
    allow_calls: u32,
    tickets: AtomicU32,
}

impl MeasuringAdapter {
    fn wrap(inner: S3MultipartAdapter, wire: Arc<AtomicU64>, allow_calls: u32) -> Arc<Self> {
        Arc::new(Self {
            inner,
            wire,
            allow_calls,
            tickets: AtomicU32::new(0),
        })
    }
}

#[async_trait::async_trait]
impl TransferAdapter for MeasuringAdapter {
    fn name(&self) -> &'static str {
        "s3_drill"
    }
    fn capabilities(&self) -> ium_core::Capability {
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
        if self.allow_calls > 0 && self.tickets.fetch_add(1, Ordering::SeqCst) >= self.allow_calls {
            // Wedged at entry: zero bytes on the wire, zero confirms in the
            // journal — the drill kills the process in exactly this state.
            std::future::pending::<()>().await;
            unreachable!()
        }
        let span = lease.end - lease.resume_from;
        let confirm = self.inner.upload_chunk(spec, lease, progress).await?;
        self.wire.fetch_add(span, Ordering::SeqCst);
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
    std::env::var("IUM_S3_ENDPOINT").expect("IUM_S3_ENDPOINT must be set (harness provides it)")
}

async fn run_upload(journal_path: &str, file: &str, allow_calls: u32) -> Result<(), String> {
    let source = std::fs::metadata(file)
        .map_err(|e| format!("source: {e}"))?
        .len();
    let spec = JobSpec {
        source_path: file.to_string(),
        source_size: source,
        adapter: "s3_drill".into(),
        endpoint: endpoint(),
    };

    let journal = SessionJournal::open(journal_path).map_err(|e| e.to_string())?;
    let mut mgr = JobManager::with_journal(journal).map_err(|e| e.to_string())?;
    let wire = Arc::new(AtomicU64::new(0));
    let inner = S3MultipartAdapter::new(S3MultipartConfig::from_env().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    mgr.register_adapter(MeasuringAdapter::wrap(
        inner,
        Arc::clone(&wire),
        allow_calls,
    ));
    let mgr = Arc::new(mgr);

    let rec = mgr
        .create_job(&spec, 4)
        .map_err(|e| format!("create_job: {e}"))?;
    println!("JOB {}", rec.id);

    // Journal-truth progress: the only thing a SIGKILL cannot take away.
    let watcher_mgr = Arc::clone(&mgr);
    let watcher_id = rec.id.clone();
    let watcher = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if let Ok(snap) = watcher_mgr.snapshot(&watcher_id) {
                let confirmed: u64 = snap.chunks.iter().map(|c| c.bytes_confirmed).sum();
                println!("CONFIRMED {confirmed}");
            }
        }
    });

    let done = mgr
        .run_job(&rec.id)
        .await
        .map_err(|e| format!("run_job: {e}"))?;
    watcher.abort();
    println!("STATE {}", done.state.as_str());
    Ok(())
}

async fn run_resume(journal_path: &str) -> Result<(), String> {
    let journal = SessionJournal::open(journal_path).map_err(|e| e.to_string())?;
    let mut mgr = JobManager::with_journal(journal).map_err(|e| e.to_string())?;
    let wire = Arc::new(AtomicU64::new(0));
    let inner = S3MultipartAdapter::new(S3MultipartConfig::from_env().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    // Crash recovery: the job sits at `Running` (or `Finishing`); find it in
    // the journal and reattach the multipart session from the journaled
    // probe event — the fresh adapter holds no session of its own.
    let jobs = mgr.list_jobs().map_err(|e| e.to_string())?;
    let rec = jobs
        .iter()
        .find(|j| !j.state.is_terminal())
        .ok_or_else(|| "no non-terminal job in the journal".to_string())?
        .clone();
    let snap = mgr.snapshot(&rec.id).map_err(|e| e.to_string())?;
    let (_, _, payload) = snap
        .events
        .iter()
        .find(|(_, k, _)| k == "probe")
        .ok_or_else(|| "no probe event in the journal".to_string())?;
    let token: serde_json::Value =
        serde_json::from_str(payload).map_err(|e| format!("probe payload: {e}"))?;
    let upload_id = token["session_token"]
        .as_str()
        .ok_or_else(|| "probe event has no session_token".to_string())?
        .to_string();
    inner.restore_session(&rec.endpoint, upload_id);

    mgr.register_adapter(MeasuringAdapter::wrap(inner, Arc::clone(&wire), 0));
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
            let allow_calls = arg_of(&args, "--allow-calls")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(run_upload(&journal, &file, allow_calls))
        }
        "resume" => tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(run_resume(&journal)),
        other => Err(format!("unknown mode '{other}' (upload|resume)")),
    };
    if let Err(e) = result {
        eprintln!("s3_drill_child: {e}");
        std::process::exit(1);
    }
}
