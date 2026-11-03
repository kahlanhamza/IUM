//! Adaptive resplit drill (issue #27 — owner question 2026-11-03: "do we
//! support splitting chunks again and again for ultimate speed on a very
//! slow connection?").
//!
//! Honest physics (stated in the issue): splitting never creates bandwidth —
//! N chunks on one slow uplink still share it. What finer parts DO buy is
//! feedback latency: the first server-confirmed byte, the first real
//! eviction vote, and less wire wasted when a connection dies mid-part. On
//! S3 lanes the protocol floor is 5 MiB per non-final part, so the win is
//! bounded — this drill pins that the bound is respected AND exploited.
//!
//! Scenario: the whole "uplink" is uniformly slow (the server paces every
//! UploadPart read at ~2 MiB/s — the impairment lives in the socket, like a
//! throttling link, exactly the #18 technique). The REAL video fixture (#17)
//! plans as ONE S3 part of ~22 MiB. At that rate the first journal-confirmed
//! byte would land after ~11 s and nothing would ever adapt — pattern #9
//! ("dynamic resizing") exists in the pool but no runtime caller drives it.
//!
//! Assertions, all from truth the engine or the wire records:
//! 1. exactly one `chunk_resplit` journal event, with REAL numbers: the
//!    fleet-median bps that triggered it, the floor-clamped target
//!    (5 MiB = CHUNK_MIN = the S3 protocol floor), and the shape change.
//! 2. the plan actually changed shape: the single ~22 MiB part becomes
//!    `floor(size / 5 MiB)` parts, every one ≥ CHUNK_MIN (the S3 floor).
//! 3. the in-flight attempt of the pre-resplit plan was abandoned mid-body
//!    (server saw the body truncate, never stored it) and part 1 completed
//!    on the retry — the resplit cost one abandoned prefix, nothing more.
//! 4. byte fidelity survives the resplit: the assembled object is the REAL
//!    video, still playable, sha256-exact; journal frontier == total size.
//! 5. honest accounting: every `chunk_confirmed` idx exists in the final
//!    plan — a resplit must never let a retired lease's confirm land on a
//!    stranger chunk.

#[allow(dead_code)] // helpers are shared with the other drills
mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use common::{assert_received_is_real_video, real_video, sha256_bytes};
use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use ium_core::eviction::{EvictionPolicy, ResplitPolicy};
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{ChunkState, JobSpec, SessionJournal};

/// The uniform "uplink" pace — every UploadPart read is throttled to this.
const RATE: u64 = 2 * 1024 * 1024; // 2 MiB/s
const READ_STEP: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// uniformly throttled S3-shaped loopback server (#18 technique, no victims:
// the WHOLE link is slow — the owner's daily reality, not one bad peer)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SrvState {
    upload_id: Mutex<Option<String>>,
    /// partNumber -> completed body (a part is atomic: complete PUT or absent).
    parts: Mutex<BTreeMap<u32, Vec<u8>>>,
    etags: Mutex<BTreeMap<u32, String>>,
    /// partNumber -> PUT attempts seen (the pre-resplit abandoned prefix
    /// must show up here as attempt 1 of part 1).
    put_counts: Mutex<BTreeMap<u32, u32>>,
    completed: Mutex<bool>,
}

struct ThrottledS3 {
    addr: std::net::SocketAddr,
    state: Arc<SrvState>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

async fn spawn_throttled_s3() -> ThrottledS3 {
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
    ThrottledS3 {
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
        let chunked = headers
            .iter()
            .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));

        let is_create = method == "POST" && query.iter().any(|(k, _)| k == "uploads");
        let part_number = query
            .iter()
            .find(|(k, _)| k == "partNumber")
            .and_then(|(_, v)| v.parse::<u32>().ok());
        let is_list_parts = method == "GET" && query.iter().any(|(k, _)| k == "uploadId");
        let is_complete = method == "POST" && query.iter().any(|(k, _)| k == "uploadId");

        if is_create {
            if content_length > 0 || chunked {
                let _ = discard_body(&mut reader, content_length, chunked).await;
            }
            *state.upload_id.lock().unwrap() = Some("ium-resplit-upload-1".into());
            let body = "<InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-resplit</Bucket><Key>real-video.mp4</Key><UploadId>ium-resplit-upload-1</UploadId></InitiateMultipartUploadResult>";
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
                handle_upload_part(&mut reader, &state, n, content_length, chunked).await;
                continue;
            }
        }

        if is_list_parts {
            let parts = state.parts.lock().unwrap().clone();
            let etags = state.etags.lock().unwrap().clone();
            let mut body = String::from(
                "<ListPartsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-resplit</Bucket><Key>real-video.mp4</Key><UploadId>ium-resplit-upload-1</UploadId><StorageClass>STANDARD</StorageClass><PartNumberMarker>0</PartNumberMarker><NextPartNumberMarker>0</NextPartNumberMarker><MaxParts>1000</MaxParts><IsTruncated>false</IsTruncated>",
            );
            for (n, bytes) in &parts {
                let etag = etags.get(n).cloned().unwrap_or_default();
                body.push_str(&format!(
                    "<Part><PartNumber>{n}</PartNumber><ETag>{etag}</ETag><Size>{}</Size></Part>",
                    bytes.len()
                ));
            }
            body.push_str("</ListPartsResult>");
            respond(
                &mut reader,
                200,
                &body,
                [("content-type", "application/xml")],
            )
            .await
            .unwrap_or(());
            continue;
        }

        if is_complete {
            // Drain the adapter's XML body BEFORE responding (keep-alive
            // desync otherwise — the #18 lesson).
            if content_length > 0 || chunked {
                let _ = discard_body(&mut reader, content_length, chunked).await;
            }
            let parts = state.parts.lock().unwrap().clone();
            let mut assembled = Vec::new();
            for bytes in parts.values() {
                assembled.extend_from_slice(bytes);
            }
            *state.completed.lock().unwrap() = true;
            let total = assembled.len();
            let body = format!(
                "<CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>http://127.0.0.1/ium-resplit/real-video.mp4</Location><Bucket>ium-resplit</Bucket><Key>real-video.mp4</Key><ETag>\"assembled-{total}\"</ETag></CompleteMultipartUploadResult>"
            );
            respond(
                &mut reader,
                200,
                &body,
                [("content-type", "application/xml")],
            )
            .await
            .unwrap_or(());
            continue;
        }

        match method.as_str() {
            "HEAD" => {
                let resp = if *state.completed.lock().unwrap() {
                    let total = state
                        .parts
                        .lock()
                        .unwrap()
                        .values()
                        .map(|b| b.len())
                        .sum::<usize>();
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {total}\r\netag: \"assembled\"\r\n\r\n"
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_string()
                };
                reader
                    .get_mut()
                    .write_all(resp.as_bytes())
                    .await
                    .unwrap_or(());
                reader.get_mut().flush().await.unwrap_or(());
            }
            _ => {
                if content_length > 0 || chunked {
                    let _ = discard_body(&mut reader, content_length, chunked).await;
                }
                let resp = "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n";
                reader
                    .get_mut()
                    .write_all(resp.as_bytes())
                    .await
                    .unwrap_or(());
                reader.get_mut().flush().await.unwrap_or(());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_upload_part(
    reader: &mut BufReader<tokio::net::TcpStream>,
    state: &Arc<SrvState>,
    part_number: u32,
    content_length: u64,
    chunked: bool,
) {
    let attempt = {
        let mut counts = state.put_counts.lock().unwrap();
        let e = counts.entry(part_number).or_insert(0);
        *e += 1;
        *e
    };
    let _ = attempt; // assertions read put_counts from the server state

    // Paced read: consume the body at RATE — the TCP receive window shrinks,
    // the client's real send slows. An EOF before the declared length means
    // the client abandoned a genuinely mid-body request (the resplit's
    // cancellation of the pre-resplit attempt).
    let mut body = Vec::new();
    let mut truncated = false;
    if chunked {
        loop {
            let mut size_line = String::new();
            if reader.read_line(&mut size_line).await.unwrap_or(0) == 0 {
                truncated = true;
                break;
            }
            let size_str = size_line.trim().split(';').next().unwrap_or("0").trim();
            let Ok(size) = usize::from_str_radix(size_str, 16) else {
                truncated = true;
                break;
            };
            if size == 0 {
                let mut t = String::new();
                while reader.read_line(&mut t).await.unwrap_or(0) > 0 && !t.trim_end().is_empty() {
                    t.clear();
                }
                break;
            }
            let mut buf = vec![0u8; size];
            if reader.read_exact(&mut buf).await.is_err() {
                truncated = true;
                break;
            }
            body.extend_from_slice(&buf);
            pace(RATE, size as u64).await;
            let mut crlf = String::new();
            reader.read_line(&mut crlf).await.unwrap_or(0);
        }
    } else {
        let mut remaining = content_length;
        while remaining > 0 {
            let step = (remaining as usize).min(READ_STEP);
            let mut buf = vec![0u8; step];
            match reader.read(&mut buf).await {
                Ok(0) => {
                    truncated = true;
                    break;
                }
                Ok(n) => {
                    body.extend_from_slice(&buf[..n]);
                    remaining -= n as u64;
                    pace(RATE, n as u64).await;
                }
                Err(_) => {
                    truncated = true;
                    break;
                }
            }
        }
    }

    if truncated {
        // The client died mid-body (the resplit cancelled its lease): the
        // attempt is never stored — a part is atomic, complete PUT or absent.
        return;
    }

    let etag = format!("\"etag-{part_number}-a{attempt}\"");
    state.parts.lock().unwrap().insert(part_number, body);
    state
        .etags
        .lock()
        .unwrap()
        .insert(part_number, etag.clone());
    let resp = format!("HTTP/1.1 200 OK\r\netag: {etag}\r\ncontent-length: 0\r\n\r\n");
    reader
        .get_mut()
        .write_all(resp.as_bytes())
        .await
        .unwrap_or(());
    reader.get_mut().flush().await.unwrap_or(());
}

async fn pace(rate_bps: u64, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let micros = bytes * 1_000_000 / rate_bps.max(1);
    tokio::time::sleep(Duration::from_micros(micros)).await;
}

async fn discard_body(
    reader: &mut BufReader<tokio::net::TcpStream>,
    content_length: u64,
    chunked: bool,
) -> std::io::Result<()> {
    if chunked {
        loop {
            let mut size_line = String::new();
            if reader.read_line(&mut size_line).await? == 0 {
                break;
            }
            let size_str = size_line.trim().split(';').next().unwrap_or("0").trim();
            let size = usize::from_str_radix(size_str, 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let mut buf = vec![0u8; size];
            reader.read_exact(&mut buf).await?;
            let mut crlf = String::new();
            reader.read_line(&mut crlf).await?;
        }
    } else if content_length > 0 {
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

// ---------------------------------------------------------------------------
// the drill
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slow_uplink_resplits_once_and_stays_byte_exact() {
    let dir = tempfile::tempdir().unwrap();
    let Some(fx) = real_video(dir.path()) else {
        return;
    };
    let srv = spawn_throttled_s3().await;

    let db = dir.path().join("journal.db");
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    // The documented test shrink (the #18 pattern): window 500 ms; floor
    // ratio and strikes stay spec-default. The resplit policy is tuned the
    // same way: a part should confirm in ~0.5 s at the observed median,
    // reshape when reality is 4× worse than that, ignore sub-10% moves.
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

    let spec = JobSpec {
        source_path: fx.path.to_string_lossy().into_owned(),
        source_size: fx.size,
        adapter: "s3_multipart".into(),
        endpoint: format!("http://{}/ium-resplit/real-video.mp4", srv.addr),
    };
    let rec = m.create_job(&spec, 1).unwrap(); // ONE part of ~22 MiB to start
    let t0 = Instant::now();
    let done = m.run_job(&rec.id).await.unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(done.state, JobState::Done, "ADR-010 verify must pass");
    assert!(
        elapsed < Duration::from_secs(60),
        "job took {elapsed:?} — sanity bound"
    );

    // -- 1. journal truth: exactly one resplit, with REAL numbers -----------
    let j = SessionJournal::open(&db).unwrap();
    let events = j.events_for(&rec.id).expect("journal events");
    let resplits: Vec<&(i64, String, String)> = events
        .iter()
        .filter(|(_, k, _)| k == "chunk_resplit")
        .collect();
    assert_eq!(
        resplits.len(),
        1,
        "exactly one resplit (hysteresis must prevent flapping); got {resplits:?}"
    );
    let payload: serde_json::Value =
        serde_json::from_str(&resplits[0].2).expect("resplit payload json");
    let median_bps = payload["median_bps"].as_f64().expect("median is real");
    assert!(
        median_bps > 0.0,
        "the resplit must record the fleet median it acted on ({median_bps})"
    );
    let target = payload["target"].as_u64().expect("target recorded");
    assert_eq!(
        target,
        5 * 1024 * 1024,
        "the implied target ({median_bps:.0} B/s × 0.5 s) collapses to the \
         CHUNK_MIN floor — the S3 5 MiB protocol minimum must be respected"
    );
    let chunks_after = payload["chunks_after"].as_u64().expect("shape recorded");
    assert_eq!(
        payload["chunks_before"].as_u64(),
        Some(1),
        "one part to start"
    );

    // -- 2. the plan actually changed shape: every piece ≥ the S3 floor ----
    //    Expected pieces: as many 5 MiB-floor-respecting pieces as the split
    //    can make (the helper must not under-split at the floor regime).
    let want_pieces = {
        let floor = 5u64 * 1024 * 1024;
        (fx.size / floor).max(1) // floor(size/floor): div_ceil would violate the floor
    };
    assert_eq!(
        chunks_after, want_pieces,
        "the single ~22 MiB part must split into floor-respecting pieces"
    );
    let rows = j.load_chunks(&rec.id).expect("chunk rows");
    assert_eq!(
        rows.len() as u64,
        want_pieces,
        "journal rows match the new plan"
    );
    assert!(
        rows.iter().all(|r| r.end - r.begin >= 5 * 1024 * 1024),
        "every piece ≥ 5 MiB (S3 non-final-part protocol floor)"
    );
    assert!(
        rows.iter()
            .all(|r| r.state == ChunkState::Uploaded || r.state == ChunkState::Verified),
        "every piece completed: {rows:?}"
    );
    let frontier: u64 = rows.iter().map(|r| r.bytes_confirmed).sum();
    assert_eq!(
        frontier, fx.size,
        "journal frontier == total size (no lost bytes)"
    );

    // -- 3. server truth: the pre-resplit attempt was abandoned mid-body ---
    let counts = srv.state.put_counts.lock().unwrap().clone();
    assert_eq!(
        counts[&1], 2,
        "part 1: the abandoned pre-resplit prefix + the retry — nothing else"
    );
    assert_eq!(
        counts.len(),
        want_pieces as usize,
        "every part of the new plan uploaded; no part skipped: {counts:?}"
    );

    // -- 4. byte fidelity survives the resplit ------------------------------
    let assembled: Vec<u8> = srv
        .state
        .parts
        .lock()
        .unwrap()
        .values()
        .flat_map(|b| b.iter().copied())
        .collect();
    assert_eq!(assembled.len() as u64, fx.size, "assembled size");
    assert_eq!(
        sha256_bytes(&assembled),
        fx.sha256,
        "assembled object is NOT the video — the resplit broke fidelity"
    );
    assert_received_is_real_video(&assembled, dir.path(), "resplit-s3");

    // -- 5. honest accounting: no confirm landed on a stranger chunk --------
    let valid_idx: std::collections::HashSet<u64> = rows.iter().map(|r| r.idx as u64).collect();
    let confirmed_idx: Vec<u64> = events
        .iter()
        .filter(|(_, k, _)| k == "chunk_confirmed")
        .filter_map(|(_, _, p)| {
            let v: serde_json::Value = serde_json::from_str(p).ok()?;
            v["idx"].as_u64()
        })
        .collect();
    assert!(
        confirmed_idx.iter().all(|i| valid_idx.contains(i)),
        "a retired lease's confirm landed on a stranger chunk: {confirmed_idx:?} \
         vs plan {valid_idx:?}"
    );

    let _ = srv.shutdown.send(());
    println!(
        "[adaptive-resplit] OK: 1 part → {chunks_after} parts on live median \
         {median_bps:.0} B/s (target floor-clamped to 5 MiB), one abandoned \
         prefix, job Done byte-exact in {elapsed:.1?} ({} bytes, sha256 {})",
        fx.size, fx.sha256
    );
}
