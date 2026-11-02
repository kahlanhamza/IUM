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
//!
//! Env: IUM_S3_ENDPOINT / IUM_S3_ACCESS_KEY / IUM_S3_SECRET_KEY /
//! IUM_S3_REGION (see `S3MultipartConfig::from_env`). Eviction policy is the
//! PRODUCTION default everywhere (10 s window, 0.4× median floor, 2 strikes)
//! — no test shrink: the real-world drill earns real-world timings.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use ium_core::eviction::EvictionPolicy;
use ium_core::state::JobState;
use ium_core::{JobManager, JobSpec, SessionJournal, Supervisor};

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
    Ok(serde_json::json!({
        "scenario": "s3hostile", "state": "Done", "size": size,
        "elapsed_s": elapsed.as_secs_f64(),
        "mean_throughput_Bps": size as f64 / elapsed.as_secs_f64(),
        "evictions": evictions,
    }))
}

async fn s4pause(journal: &str, file: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    let m = fresh_manager(journal)?;
    let sup = Supervisor::new(Arc::clone(&m));
    let spec = spec_for(file, endpoint)?;
    let size = spec.source_size;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    sup.drive(&rec.id);

    // pause once ≥10% is confirmed (whole parts are atomic, so the frontier
    // moves in part-sized steps)
    let frontier_at_pause = loop {
        let f = frontier_of(&m, &rec.id)?;
        if f * 10 >= size {
            sup.pause(&rec.id).map_err(|e| e.to_string())?;
            break f;
        }
        if !sup.is_driving(&rec.id) {
            // drive exited without pause landing (too-fast upload): fail loud
            return Err(
                "s4pause: job reached a terminal state before the pause landed \
                        (upload too fast for the 10% frontier) — rerun on a slower path"
                    .into(),
            );
        }
        if t0.elapsed() > Duration::from_secs(120) {
            return Err("s4pause: timeout waiting for the 10% frontier".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let paused_state = m.snapshot(&rec.id).map_err(|e| e.to_string())?.job.state;
    if paused_state != JobState::Paused {
        return Err(format!(
            "s4pause: state after pause is {paused_state:?} (must be Paused)"
        ));
    }
    // the wire must be frozen: 2.5 s later the frontier is byte-identical
    tokio::time::sleep(Duration::from_secs(2) + Duration::from_millis(500)).await;
    let frontier_frozen = frontier_of(&m, &rec.id)?;
    if frontier_frozen != frontier_at_pause {
        return Err(format!(
            "s4pause: frontier moved while Paused ({frontier_at_pause} → {frontier_frozen})"
        ));
    }
    sup.resume_signal(&rec.id).map_err(|e| e.to_string())?;
    let final_state = wait_terminal(&m, &sup, &rec.id, Duration::from_secs(600)).await?;
    if final_state != JobState::Done {
        return Err(format!("s4pause: resumed job ended {final_state:?}"));
    }
    Ok(serde_json::json!({
        "scenario": "s4pause", "paused_state": "Paused",
        "frontier_at_pause": frontier_at_pause,
        "frontier_after_2_5s": frontier_frozen,
        "final_state": "Done",
        "size": size, "total_elapsed_s": t0.elapsed().as_secs_f64(),
    }))
}

async fn s4cancel(journal: &str, file: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    let m = fresh_manager(journal)?;
    let sup = Supervisor::new(Arc::clone(&m));
    let spec = spec_for(file, endpoint)?;
    let rec = m.create_job(&spec, 4).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    sup.drive(&rec.id);
    // cancel once ≥1 whole part is confirmed (frontier > 0)
    let frontier_at_cancel = loop {
        let f = frontier_of(&m, &rec.id)?;
        if f > 0 {
            break f;
        }
        if !sup.is_driving(&rec.id) {
            return Err("s4cancel: job went terminal before any part confirmed".into());
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
        other => Err(format!(
            "unknown scenario '{other}' (s1|s3hostile|s4pause|s4cancel|s5)"
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
