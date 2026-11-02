//! Real slow-connection drill (issue #18 — owner mandate 2026-11-02: real
//! slow-connection testing). Spec §4.2's eviction promise, fired on LIVE
//! wire evidence for the first time:
//!
//! * A local S3-shaped loopback server (test code; the same request shapes
//!   the adapter pins against Garage/SeaweedFS) paces its socket READS. Slow
//!   reads shrink the TCP receive window, so the production adapter's real
//!   streaming send genuinely slows — the impairment lives in the socket,
//!   like a throttling link, not in a test clock. The server selects the
//!   victim deterministically: part 2's FIRST UploadPart reads at ~96 KiB/s,
//!   every other PUT at ~2 MiB/s; a retry (second PUT for that partNumber)
//!   is full speed — the aria2 model: abandon the impaired connection, the
//!   re-established one is fast.
//! * The engine runs the REAL `S3MultipartAdapter` (real SigV4) with
//!   `max_conns=3` → a real 3-worker fleet, the only lane that can evict
//!   (HTTP one-shot and TUS #13 are single-worker by design).
//! * Data: the REAL video fixture (#17) — the mandate is a video on a slow
//!   connection, not zeros.
//!
//! Assertions, all from truth the engine or the wire itself records:
//! 1. journal `chunk_evicted` event with REAL numbers: measured bps below
//!    0.4 × measured pool median (eviction policy window 500 ms — the
//!    documented test shrink; strikes and ratios stay spec-default).
//! 2. job ends `Done` via ADR-010 verify, in ≪ wait-out time (a full slow
//!    PUT would need ~80 s; the bound here is 30 s — the bound itself is
//!    proof the impaired connection was abandoned mid-body).
//! 3. server truth: the impaired attempt STARTED and never completed — it is
//!    still in flight (head read, body never finished) when the job is Done,
//!    while its RETRY completed the span; every part completed exactly once.
//!    (The abandonment is observable as in-flight-not-completed because hyper
//!    lingers the abandoned pooled socket — no FIN arrives inside the window.)
//! 4. byte fidelity survives the eviction: the assembled object equals the
//!    source sha256 and is still a PLAYABLE video (S3 parts confirm nothing
//!    partial, so the re-PUT re-sent the whole span — protocol-honest).

#[allow(dead_code)] // helpers are shared with tests/real_data.rs
mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use common::{assert_received_is_real_video, real_video, sha256_bytes};
use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use ium_core::eviction::EvictionPolicy;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{JobSpec, SessionJournal};

/// The victim: partNumber 2 (idx 1) is throttled on its first attempt only.
const SLOW_PART: u32 = 2;
const SLOW_RATE: u64 = 96 * 1024; // bytes/sec the impaired "link" carries
const FAST_RATE: u64 = 2 * 1024 * 1024; // healthy connections' pace
const READ_STEP: usize = 16 * 1024;

// ---------------------------------------------------------------------------
// throttled S3-shaped loopback server
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SrvState {
    upload_id: Mutex<Option<String>>,
    /// partNumber -> completed body (a part is atomic: complete PUT or absent).
    parts: Mutex<BTreeMap<u32, Vec<u8>>>,
    etags: Mutex<BTreeMap<u32, String>>,
    /// partNumber -> PUT attempts seen (the throttle applies to attempt 1 of
    /// the victim only).
    put_counts: Mutex<BTreeMap<u32, u32>>,
    /// Requests that STARTED (head read) but never completed their body.
    /// The eviction abandons the impaired request mid-body; hyper lingers
    /// the pooled socket (no FIN inside the test window), so the honest
    /// server-side observable of the abandonment is: still in flight while
    /// the job completes on a healthy retry.
    in_flight: Mutex<Vec<(u32, u32)>>,
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

        // --- route ---------------------------------------------------------
        let is_create = method == "POST" && query.iter().any(|(k, _)| k == "uploads");
        let part_number = query
            .iter()
            .find(|(k, _)| k == "partNumber")
            .and_then(|(_, v)| v.parse::<u32>().ok());
        let is_list_parts = method == "GET" && query.iter().any(|(k, _)| k == "uploadId");
        let is_complete = method == "POST" && query.iter().any(|(k, _)| k == "uploadId");

        if is_create {
            // Consume the request body (CreateMultipartUpload's is empty, but
            // a keep-alive connection MUST see its body drained before the
            // next request line, or the connection desyncs).
            if content_length > 0 || chunked {
                let _ = discard_body(&mut reader, content_length, chunked).await;
            }
            *state.upload_id.lock().unwrap() = Some("ium-slow-upload-1".into());
            let body = "<InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-slow</Bucket><Key>real-video.mp4</Key><UploadId>ium-slow-upload-1</UploadId></InitiateMultipartUploadResult>";
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
                "<ListPartsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>ium-slow</Bucket><Key>real-video.mp4</Key><UploadId>ium-slow-upload-1</UploadId><StorageClass>STANDARD</StorageClass><PartNumberMarker>0</PartNumberMarker><NextPartNumberMarker>0</NextPartNumberMarker><MaxParts>1000</MaxParts><IsTruncated>false</IsTruncated>",
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
            // The adapter PUTs a real XML body here — drain it BEFORE
            // responding, or the keep-alive connection desyncs and the next
            // request (verify's HEAD) reads leftover XML as its request line.
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
                "<CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>http://{}/ium-slow/real-video.mp4</Location><Bucket>ium-slow</Bucket><Key>real-video.mp4</Key><ETag>\"assembled-{total}\"</ETag></CompleteMultipartUploadResult>",
                state_addr()
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
                if *state.completed.lock().unwrap() {
                    let total = state
                        .parts
                        .lock()
                        .unwrap()
                        .values()
                        .map(|b| b.len())
                        .sum::<usize>();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {total}\r\netag: \"assembled\"\r\n\r\n"
                    );
                    reader
                        .get_mut()
                        .write_all(resp.as_bytes())
                        .await
                        .unwrap_or(());
                    reader.get_mut().flush().await.unwrap_or(());
                } else {
                    let resp = "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n";
                    reader
                        .get_mut()
                        .write_all(resp.as_bytes())
                        .await
                        .unwrap_or(());
                    reader.get_mut().flush().await.unwrap_or(());
                }
            }
            "GET" => {
                if *state.completed.lock().unwrap() {
                    let assembled: Vec<u8> = state
                        .parts
                        .lock()
                        .unwrap()
                        .values()
                        .flat_map(|b| b.iter().copied())
                        .collect();
                    let head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: video/mp4\r\n\r\n",
                        assembled.len()
                    );
                    let mut out = head.into_bytes();
                    out.extend_from_slice(&assembled);
                    reader.get_mut().write_all(&out).await.unwrap_or(());
                    reader.get_mut().flush().await.unwrap_or(());
                } else {
                    let resp = "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n";
                    reader
                        .get_mut()
                        .write_all(resp.as_bytes())
                        .await
                        .unwrap_or(());
                    reader.get_mut().flush().await.unwrap_or(());
                }
            }
            _ => {
                // read+discard any body, then 405
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

fn state_addr() -> String {
    "127.0.0.1".to_string()
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
    let throttled = part_number == SLOW_PART && attempt == 1;
    let rate = if throttled { SLOW_RATE } else { FAST_RATE };
    state.in_flight.lock().unwrap().push((part_number, attempt));

    let mut body = Vec::new();
    let mut truncated = false;
    if chunked {
        // The adapter always declares content-length for UploadPart, but the
        // drill reads chunked bodies too so the throttle is honest in both
        // framings.
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
            pace(rate, size as u64).await;
            let mut crlf = String::new();
            reader.read_line(&mut crlf).await.unwrap_or(0);
        }
    } else {
        // Paced read: consume the body at `rate` — the TCP receive window
        // shrinks, the client's real send slows. An EOF before the declared
        // length means the client abandoned a genuinely mid-body request.
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
                    pace(rate, n as u64).await;
                }
                Err(_) => {
                    truncated = true;
                    break;
                }
            }
        }
    }

    if truncated {
        // The client died mid-body (RST/FIN): count as abandoned too.
        let mut f = state.in_flight.lock().unwrap();
        if let Some(pos) = (0..f.len()).rev().find(|&i| f[i] == (part_number, attempt)) {
            f.remove(pos);
        }
        return; // connection is dead; no response possible
    }

    let etag = format!("\"etag-{part_number}-a{attempt}\"");
    state.parts.lock().unwrap().insert(part_number, body);
    state
        .etags
        .lock()
        .unwrap()
        .insert(part_number, etag.clone());
    {
        let mut f = state.in_flight.lock().unwrap();
        if let Some(pos) = (0..f.len()).rev().find(|&i| f[i] == (part_number, attempt)) {
            f.remove(pos);
        }
    }
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
async fn slow_connection_is_evicted_on_live_wire_throughput() {
    let dir = tempfile::tempdir().unwrap();
    let Some(fx) = real_video(dir.path()) else {
        return;
    };
    let srv = spawn_throttled_s3().await;

    let db = dir.path().join("journal.db");
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    // The documented test shrink: window 500 ms; floor ratio (0.4×median) and
    // 2-consecutive-strikes stay at their spec §4.2 defaults.
    m.set_eviction_policy(EvictionPolicy {
        window: Duration::from_millis(500),
        floor_ratio: 0.4,
        strikes_to_evict: 2,
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
        endpoint: format!("http://{}/ium-slow/real-video.mp4", srv.addr),
    };
    let rec = m.create_job(&spec, 3).unwrap(); // real 3-worker fleet
    let t0 = Instant::now();
    let done = m.run_job(&rec.id).await.unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(done.state, JobState::Done, "ADR-010 verify must pass");

    // -- 2. the time bound is itself the proof: waiting out a full slow PUT
    //       (~7.7 MiB at 96 KiB/s) needs ~80 s. The job must be long done.
    assert!(
        elapsed < Duration::from_secs(30),
        "job took {elapsed:?} — the slow connection was waited out, not evicted"
    );

    // -- 1. journal truth: exactly one eviction, with REAL wire numbers.
    let j = SessionJournal::open(&db).unwrap();
    let events = j.events_for(&rec.id).expect("journal events");
    let evictions: Vec<&(i64, String, String)> = events
        .iter()
        .filter(|(_, k, _)| k == "chunk_evicted")
        .collect();
    assert_eq!(
        evictions.len(),
        1,
        "exactly the impaired part is evicted; got {evictions:?}"
    );
    let payload: serde_json::Value =
        serde_json::from_str(&evictions[0].2).expect("eviction payload json");
    let bps = payload["bps"].as_f64().expect("bps is real");
    let median_bps = payload["median_bps"].as_f64().expect("median is real");
    assert!(
        bps < 0.4 * median_bps,
        "the evicted worker's measured throughput ({bps} B/s) must sit below the \
         floor (0.4 × median {median_bps} B/s) — these are wire measurements"
    );
    assert_eq!(
        payload["idx"].as_u64().unwrap() as u32 + 1,
        SLOW_PART,
        "the evicted chunk is the throttled part"
    );

    // -- 3. server truth: the impaired request was abandoned mid-body and
    //       never contributed bytes; the retry completed the span.
    let in_flight = srv.state.in_flight.lock().unwrap().clone();
    assert_eq!(
        in_flight,
        vec![(SLOW_PART, 1)],
        "exactly the throttled attempt is still in flight (abandoned mid-body) \
         when the job completes: {in_flight:?}"
    );
    let counts = srv.state.put_counts.lock().unwrap().clone();
    assert_eq!(counts[&SLOW_PART], 2, "one eviction, one clean retry");
    assert_eq!(counts.len(), 3, "every part uploaded; no part skipped");
    let etags = srv.state.etags.lock().unwrap().clone();
    assert!(
        etags[&SLOW_PART].contains("a2"),
        "the completed part 2 came from the RETRY, not the impaired attempt"
    );

    // -- 4. byte fidelity survives the eviction: the assembled object is the
    //       real video, still playable.
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
        "assembled object is NOT the video — eviction broke fidelity"
    );
    assert_received_is_real_video(&assembled, dir.path(), "slow-s3");

    let _ = srv.shutdown.send(());
    println!(
        "[slow-connection] OK: evicted part {SLOW_PART} at {bps:.0} B/s (pool median \
         {median_bps:.0} B/s, floor 0.4×median, 2 strikes) after {elapsed:.1?}; the impaired \
         attempt was abandoned mid-body, the retry completed the span; job Done, video intact \
         ({} bytes, sha256 {})",
        fx.size, fx.sha256
    );
}
