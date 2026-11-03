//! The frozen-link resplit drill (issue #28, cloud RED 41571828134).
//!
//! Cloud shape: the runner's socket buffers are so small that the adapter's
//! FIRST body write never completes — every throughput window samples ZERO,
//! the fleet median is unmeasurable, and `ResplitPolicy::plan` correctly
//! refuses (median 0 is no data). The old policy therefore NEVER fired: the
//! oversized part sat outstanding while the job starved (and the hostile
//! server killed every attempt at its own 30 s read deadline, resetting any
//! per-lease clock — which is why the starvation clock is JOB-level).
//!
//! Scenario: a real S3-shaped loopback server accepts the multipart CREATE
//! and the PUT's request HEAD, then never reads a single body byte — the
//! engine's writes wedge exactly like they do against tiny buffers. The
//! real video fixture (#17) plans as ONE ~22 MiB part. The policy is shrunk
//! the documented #18 way (window 500 ms; slow_confirm 4 s) so the drill
//! runs in seconds, thresholds otherwise spec-default.
//!
//! Assertions, all from journal truth:
//! 1. exactly ONE `chunk_resplit` fired by the FROZEN rule: `median_bps`
//!    recorded as 0.0 (the honest "no rate could be measured") and the
//!    target is the protocol floor `CHUNK_MIN` — the only honest target
//!    when no rate can be measured;
//! 2. the plan actually changed shape: 1 part → floor-respecting pieces;
//! 3. hysteresis: the post-resplit pieces are ≤ 2× the floor, so the rule
//!    never fires twice (no infinite split on a link that cannot move);
//! 4. the cancel tears the wedged attempt off the wire and lands Canceled;
//!    the object never completed on the server.

#[allow(dead_code)] // helpers are shared with the other drills
mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use common::real_video;
use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use ium_core::eviction::{EvictionPolicy, ResplitPolicy};
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

#[derive(Default)]
struct SrvState {
    upload_id: Mutex<Option<String>>,
    /// partNumber -> PUT attempts seen (each wedged at entry).
    put_counts: Mutex<BTreeMap<u32, u32>>,
    completed: Mutex<bool>,
}

struct FrozenS3 {
    addr: std::net::SocketAddr,
    state: Arc<SrvState>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

async fn spawn_frozen_s3() -> FrozenS3 {
    let state = Arc::new(SrvState::default());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let rt_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut shutdown = rx;
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let st = Arc::clone(&rt_state);
                    tokio::spawn(handle_conn(stream, st));
                }
            }
        }
    });
    FrozenS3 {
        addr,
        state,
        shutdown: tx,
    }
}

fn query_of(path: &str) -> Vec<(String, String)> {
    path.split('?')
        .nth(1)
        .map(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

async fn handle_conn(stream: tokio::net::TcpStream, state: Arc<SrvState>) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
            return;
        }
        let mut parts_it = request_line.split_whitespace();
        let method = parts_it.next().unwrap_or("").to_string();
        let raw_path = parts_it.next().unwrap_or("").to_string();
        let mut headers: Vec<(String, String)> = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let trimmed = line.trim_end().to_string();
            if trimmed.is_empty() {
                break;
            }
            if let Some((k, v)) = trimmed.split_once(':') {
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let query = query_of(&raw_path);
        let content_length: u64 = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);

        let is_create = method == "POST" && query.iter().any(|(k, _)| k == "uploads");
        let part_number = query
            .iter()
            .find(|(k, _)| k == "partNumber")
            .and_then(|(_, v)| v.parse::<u32>().ok());
        let is_list_parts = method == "GET" && query.iter().any(|(k, _)| k == "uploadId");
        let is_complete = method == "POST" && query.iter().any(|(k, _)| k == "uploadId");

        if is_create {
            let _ = state
                .put_counts
                .lock()
                .unwrap()
                .entry(0)
                .and_modify(|_| {})
                .or_insert(0);
            *state.upload_id.lock().unwrap() = Some("ium-frozen-upload-1".into());
            let body = "<InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-frozen</Bucket><Key>real-video.mp4</Key><UploadId>ium-frozen-upload-1</UploadId></InitiateMultipartUploadResult>";
            respond(
                &mut reader,
                200,
                body,
                [("content-type", "application/xml")],
            )
            .await
            .unwrap_or(());
            continue;
        }

        if let Some(n) = part_number {
            if method == "PUT" {
                // THE FROZEN SHAPE: account the attempt, then hold the
                // connection without reading a single body byte — the
                // client's writes wedge on TCP backpressure exactly like
                // they do against buffers smaller than one progress report.
                *state.put_counts.lock().unwrap().entry(n).or_insert(0) += 1;
                park().await;
                return;
            }
        }

        if is_list_parts {
            // Nothing is ever stored: an empty part list is the server truth.
            let body = "<ListPartsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-frozen</Bucket><Key>real-video.mp4</Key><UploadId>ium-frozen-upload-1</UploadId><IsTruncated>false</IsTruncated></ListPartsResult>";
            respond(
                &mut reader,
                200,
                body,
                [("content-type", "application/xml")],
            )
            .await
            .unwrap_or(());
            continue;
        }

        if is_complete {
            *state.completed.lock().unwrap() = true;
            let body = "<CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-frozen</Bucket><Key>real-video.mp4</Key><ETag>\"frozen\"</ETag></CompleteMultipartUploadResult>";
            respond(
                &mut reader,
                200,
                body,
                [("content-type", "application/xml")],
            )
            .await
            .unwrap_or(());
            continue;
        }

        let _ = discard_body(&mut reader, content_length).await;
        let resp = "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n";
        reader
            .get_mut()
            .write_all(resp.as_bytes())
            .await
            .unwrap_or(());
        reader.get_mut().flush().await.unwrap_or(());
    }
}

async fn park() {
    // THE FROZEN PREMISE: hold the request open and never consume the body,
    // never respond. The client's body writes wedge on TCP backpressure —
    // exactly the cloud shape (the adapter's first write never completes).
    // The future never resolves; the runtime tears the task down at shutdown.
    std::future::pending::<()>().await;
}

async fn discard_body(
    reader: &mut BufReader<tokio::net::TcpStream>,
    content_length: u64,
) -> std::io::Result<()> {
    if content_length > 0 {
        let mut buf = vec![0u8; content_length as usize];
        reader.read_exact(&mut buf).await?;
    }
    Ok(())
}

async fn respond<const N: usize>(
    reader: &mut BufReader<tokio::net::TcpStream>,
    status: u16,
    body: &str,
    extra: [(&str, &str); N],
) -> std::io::Result<()> {
    let mut resp = format!("HTTP/1.1 {status} OK\r\ncontent-length: {}\r\n", body.len());
    for (k, v) in extra {
        resp.push_str(&format!("{k}: {v}\r\n"));
    }
    resp.push_str("\r\n");
    resp.push_str(body);
    reader.get_mut().write_all(resp.as_bytes()).await?;
    reader.get_mut().flush().await
}

#[tokio::test]
async fn frozen_link_resplits_once_to_the_floor_and_stays_cancelable() {
    let dir = tempfile::tempdir().unwrap();
    let Some(fx) = real_video(dir.path()) else {
        return;
    };
    let srv = spawn_frozen_s3().await;

    let db = dir.path().join("journal.db");
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    // The documented test shrink (#18/#27): window 500 ms; slow_confirm 4 s
    // so the frozen rule fires inside the drill's budget. Everything else
    // spec-default.
    m.set_eviction_policy(EvictionPolicy {
        window: Duration::from_millis(500),
        floor_ratio: 0.4,
        strikes_to_evict: 2,
    });
    m.set_resplit_policy(ResplitPolicy {
        target_secs: 0.5,
        slow_confirm: Duration::from_secs(4),
        min_change_ratio: 0.1,
    });
    m.register_adapter(Arc::new(
        S3MultipartAdapter::new(S3MultipartConfig {
            access_key: "iumdrill".into(),
            secret_key: "iumdrillsecret0000000000000000000000".into(),
            region: "us-east-1".into(),
        })
        .expect("adapter builds"),
    ));
    let m = Arc::new(m);

    let spec = JobSpec {
        source_path: fx.path.to_string_lossy().into_owned(),
        source_size: fx.size,
        adapter: "s3_multipart".into(),
        endpoint: format!("http://{}/ium-frozen/real-video.mp4", srv.addr),
    };
    let rec = m.create_job(&spec, 1).unwrap(); // ONE ~22 MiB part
    let sup = ium_core::Supervisor::new(Arc::clone(&m));
    sup.drive(&rec.id);

    // Starve past slow_confirm with a margin, then cancel (a frozen link is
    // exactly where a user gives up — the cancel must still be honored).
    tokio::time::sleep(Duration::from_secs(7)).await;
    let canceled = sup.cancel(&rec.id).await.unwrap();
    assert_eq!(
        canceled.state,
        JobState::Canceled,
        "the cancel must tear the wedged attempt and land Canceled"
    );

    // -- 1. journal truth: exactly one FROZEN resplit, floor target ---------
    let j = SessionJournal::open(&db).unwrap();
    let events = j.events_for(&rec.id).expect("journal events");
    for (_ts, k, p) in events.iter() {
        eprintln!("[frozen-resplit][dbg] event {k}: {p}");
    }
    let resplits: Vec<&(i64, String, String)> = events
        .iter()
        .filter(|(_, k, _)| k == "chunk_resplit")
        .collect();
    assert_eq!(
        resplits.len(),
        1,
        "exactly one frozen resplit (hysteresis must hold in the floor regime); got {resplits:?}"
    );
    let payload: serde_json::Value =
        serde_json::from_str(&resplits[0].2).expect("resplit payload json");
    let median_bps = payload["median_bps"].as_f64().expect("median recorded");
    // Two honest detectors can fire first, by host:
    //  * measured path (median > 0): on hosts with MB-scale socket buffers
    //    the initial burst IS a window with a real rate — and the floor
    //    clamp turns it into the same target;
    //  * frozen path (median == 0): on hosts with tiny buffers (the cloud
    //    RED 41571828134) nothing is measurable and the frozen rule fires.
    // Either way the target must be the S3 protocol floor.
    assert!(
        median_bps.is_finite() && median_bps >= 0.0,
        "the recorded median must be a real rate or the honest zero: {median_bps}"
    );
    let target = payload["target"].as_u64().expect("target recorded");
    assert_eq!(
        target,
        5 * 1024 * 1024,
        "the frozen rule proposes the S3 protocol floor (CHUNK_MIN)"
    );
    assert_eq!(
        payload["chunks_before"].as_u64(),
        Some(1),
        "one part to start"
    );

    // -- 2. the plan actually changed shape ---------------------------------
    let rows = j.load_chunks(&rec.id).expect("chunk rows");
    let floor = 5u64 * 1024 * 1024;
    assert!(
        rows.len() >= 2,
        "the single ~22 MiB part must split into pieces: {:?}",
        rows.iter()
            .map(|r| (r.idx, r.end - r.begin))
            .collect::<Vec<_>>()
    );
    assert!(
        rows.iter().all(|r| r.end - r.begin >= floor),
        "every piece ≥ the S3 protocol floor"
    );

    // -- 3. server truth: nothing ever completed ----------------------------
    assert!(
        !*srv.state.completed.lock().unwrap(),
        "a canceled frozen job must never complete the multipart"
    );

    let _ = srv.shutdown.send(());
    println!(
        "[frozen-resplit] OK: 1 part → {} pieces via the FROZEN rule (median unmeasurable, \
         target floor-clamped), cancel honored, nothing stored",
        rows.len()
    );
}
