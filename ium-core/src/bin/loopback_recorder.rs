//! The HTTP loopback recorder, as a process (issue #35).
//!
//! The app e2e needs a REAL server the shell can upload to through the real
//! engine: PUT records the body and answers 200 + ETag, HEAD reports the
//! bytes actually held — and 404 when NOTHING is held, an object store's
//! honest "no such object" (the cancel leg asserts exactly that: a canceled
//! upload never completed the object). Body reads can be throttled so the
//! e2e controls the clock instead of racing it (the #28/#32 lesson).
//!
//! Deliberately a SELF-CONTAINED binary: `tests/http_one_shot.rs` keeps its
//! own in-process copy pinned to the wire suite — no shared code, no
//! drive-by refactor; the recorder serves the SHELL seam, the test copy
//! serves the adapter seam.
//!
//! Usage:
//! ```text
//! loopback-recorder --port 22390 \
//!     [--throttle-bps 2097152] \
//!     [--record-file /tmp/e2e/recorder.json]
//! ```
//!
//! `--record-file` is rewritten after every request with
//! `{"put_count": n, "received_bytes": n, "tus_sessions": n,
//! "tus_bytes": n, "tus_last": "<id>|null"}` — the e2e reads it to assert
//! on what the server really holds without needing a second protocol.
//!
//! TUS lane (issue #37 CI debt: the shell e2e's live-progress leg needs a
//! transport whose `bytes_confirmed` advances DURING the transfer — the
//! one-shot PUT family confirms only at completion, so the UI percent can
//! honestly never move mid-flight). The lane speaks the exact subset the
//! `TusAdapter` rehearses against tusd (wire facts in `adapter_tus.rs`):
//! * `POST /files` + `Upload-Length` + `Tus-Resumable: 1.0.0` → `201` +
//!   `Location: /files/<id>` (relative — the adapter resolves both forms);
//! * `HEAD /files/<id>` → `200` + `Upload-Offset` + `Upload-Length`
//!   (`404` unknown — the adapter's SessionLost shape);
//! * `PATCH /files/<id>` + `Upload-Offset` +
//!   `Content-Type: application/offset+octet-stream` → `204` + the NEW
//!   offset (the server-acked truth); wrong offset → `409` after a fast
//!   body drain (tusd-like: the client sees the status, not a reset); a
//!   wrong/missing `Tus-Resumable` → `412` (tusd's precondition gate).
//!
//! PATCH body reads are throttled like PUT body reads — the e2e controls
//! the clock, so per-PATCH server acks land a few seconds apart and the
//! UI's percent visibly climbs chunk by chunk.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Default)]
struct RecorderState {
    /// Bytes actually committed across all accepted PUT bodies.
    received: Vec<u8>,
    /// PUTs that crossed the wire with a fully-read body.
    put_count: u64,
    /// TUS sessions keyed by id (`/files/<id>`), each lock separate so a
    /// HEAD during a long throttled PATCH never blocks on it.
    tus: std::collections::HashMap<String, Arc<std::sync::Mutex<TusSession>>>,
    /// Monotonic session id source.
    tus_next: u64,
    /// Id of the most recently created session (the record file surfaces
    /// it so the e2e can HEAD the real session URL without guessing).
    tus_last: Option<String>,
}

/// One TUS upload resource: the declared length and the bytes the server
/// actually holds (durable only up to this process, exactly like tusd's
/// file store on a tmpfs — the e2e's lifetime is the process's).
struct TusSession {
    length: u64,
    buf: Vec<u8>,
}

struct Args {
    port: u16,
    throttle_bps: Option<u64>,
    record_file: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut port = 22390;
    let mut throttle_bps = None;
    let mut record_file = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| die("--port needs a number"));
            }
            "--throttle-bps" => {
                throttle_bps = Some(
                    args.next()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| die("--throttle-bps needs a number")),
                );
            }
            "--record-file" => {
                record_file = Some(PathBuf::from(
                    args.next()
                        .unwrap_or_else(|| die("--record-file needs a path")),
                ));
            }
            other => die(&format!("unknown argument: {other}")),
        }
    }
    Args {
        port,
        throttle_bps,
        record_file,
    }
}

fn die(msg: &str) -> ! {
    eprintln!("[loopback-recorder] fatal: {msg}");
    std::process::exit(2);
}

/// Rewrite the record file after every request — the e2e's server-truth
/// surface. Best-effort: a failed write is loud on stderr, never fatal.
fn write_record(path: &Option<PathBuf>, state: &RecorderState) {
    let Some(path) = path else { return };
    let tus_bytes: usize = state
        .tus
        .values()
        .map(|s| s.lock().unwrap().buf.len())
        .sum();
    let tus_last = state
        .tus_last
        .as_deref()
        .map(|id| format!("\"{id}\""))
        .unwrap_or_else(|| "null".into());
    let json = format!(
        "{{\"put_count\": {}, \"received_bytes\": {}, \"tus_sessions\": {}, \
         \"tus_bytes\": {}, \"tus_last\": {}}}\n",
        state.put_count,
        state.received.len(),
        state.tus.len(),
        tus_bytes,
        tus_last
    );
    if let Err(e) = std::fs::write(path, json) {
        eprintln!("[loopback-recorder] record write failed: {e}");
    }
}

/// Paced body read: sleeps between chunks so a PUT of B bytes takes
/// approximately B / throttle seconds on the wire clock. Reads arrive in
/// 64 KiB steps — fine-grained enough for a mid-body cancel to land while
/// bytes are still in flight.
async fn read_body_throttled(
    reader: &mut BufReader<tokio::net::TcpStream>,
    len: u64,
    throttle_bps: Option<u64>,
) -> std::io::Result<Vec<u8>> {
    const CHUNK: usize = 64 * 1024;
    let mut body = Vec::with_capacity(len as usize);
    let mut remaining = len;
    while remaining > 0 {
        let step = remaining.min(CHUNK as u64) as usize;
        let off = body.len();
        body.resize(off + step, 0);
        reader.read_exact(&mut body[off..]).await?;
        remaining -= step as u64;
        if let Some(bps) = throttle_bps {
            let secs = (CHUNK as f64) / (bps as f64);
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }
    }
    Ok(body)
}

async fn read_chunked_throttled(
    reader: &mut BufReader<tokio::net::TcpStream>,
    throttle_bps: Option<u64>,
) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line).await?;
        let size_str = size_line.trim().split(';').next().unwrap_or("0").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if size == 0 {
            loop {
                let mut t = String::new();
                let n = reader.read_line(&mut t).await?;
                if n == 0 || t.trim_end().is_empty() {
                    break;
                }
            }
            break;
        }
        let off = body.len();
        body.resize(off + size, 0);
        reader.read_exact(&mut body[off..]).await?;
        if let Some(bps) = throttle_bps {
            let secs = (size as f64) / (bps as f64);
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }
        let mut crlf = String::new();
        reader.read_line(&mut crlf).await?;
    }
    Ok(body)
}

async fn handle_conn(
    stream: tokio::net::TcpStream,
    state: Arc<Mutex<RecorderState>>,
    throttle_bps: Option<u64>,
    record_file: Option<PathBuf>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    loop {
        // ---- request head -------------------------------------------------
        let mut request_line = String::new();
        let n = reader.read_line(&mut request_line).await?;
        if n == 0 {
            return Ok(()); // client closed
        }
        let mut headers: Vec<(String, String)> = Vec::new();
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(());
            }
            let trimmed = line.trim_end().to_string();
            if trimmed.is_empty() {
                break;
            }
            if let Some((k, v)) = trimmed.split_once(':') {
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("/").split('?').next().unwrap_or("/");
        // Everything under the /files namespace is the TUS lane — including
        // malformed session paths like `/filesdoes-not-exist` (the adapter's
        // bogus-session drill concatenates without a slash): a conforming
        // server answers those 404 from the TUS lane, never 405 from PUT/HEAD.
        let is_tus = target.starts_with("/files");

        // ---- TUS lane: handlers own their body reads (a PATCH's offset
        // decides whether the body is consumed at all, mirroring tusd). ----
        let response = if is_tus {
            handle_tus(
                &method,
                target,
                &headers,
                &mut reader,
                &state,
                throttle_bps,
                &record_file,
            )
            .await?
        } else {
            // ---- legacy PUT/HEAD lane: body then dispatch -------------------
            // A connection that dies mid-body (the cancel leg: the engine drops
            // the PUT when the user cancels) surfaces as a read error here —
            // nothing is committed, exactly like a real object store that only
            // accepts a fully-delivered object.
            let body = if headers
                .iter()
                .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"))
            {
                read_chunked_throttled(&mut reader, throttle_bps).await?
            } else if let Some((_, cl)) = headers.iter().find(|(k, _)| k == "content-length") {
                let len: u64 = cl.parse().unwrap_or(0);
                read_body_throttled(&mut reader, len, throttle_bps).await?
            } else {
                Vec::new()
            };
            match method.as_str() {
                "PUT" => {
                    {
                        let mut st = state.lock().unwrap();
                        st.received.extend_from_slice(&body);
                        st.put_count += 1;
                    }
                    write_record(&record_file, &state.lock().unwrap());
                    eprintln!(
                        "[loopback-recorder] PUT committed {} bytes (total {})",
                        body.len(),
                        state.lock().unwrap().received.len()
                    );
                    "HTTP/1.1 200 OK\r\netag: \"ium-e2e\"\r\ncontent-length: 0\r\n\r\n".to_string()
                }
                "HEAD" => {
                    let held = state.lock().unwrap().received.len();
                    eprintln!("[loopback-recorder] HEAD held={held}");
                    if held == 0 {
                        // The honest "no object" — the cancel leg pins it.
                        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_string()
                    } else {
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {held}\r\n\r\n")
                    }
                }
                _ => "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n".to_string(),
            }
        };
        reader.get_mut().write_all(response.as_bytes()).await?;
        reader.get_mut().flush().await?;
    }
}

/// The TUS lane's version gate (tusd answers 412 when the client's
/// `Tus-Resumable` is missing or unsupported — the adapter always sends
/// `1.0.0`, so a 412 here means a client bug, which the e2e wants loud).
fn tus_version_ok(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(k, v)| k == "tus-resumable" && v.trim() == "1.0.0")
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.trim())
}

/// Drain a PATCH body fast (no throttle): a rejected PATCH's flush is
/// waste by definition — tusd drains so the client sees the status line
/// instead of a connection reset (the #33/#37 wire fact in `adapter_tus`).
async fn drain_body(
    reader: &mut BufReader<tokio::net::TcpStream>,
    headers: &[(String, String)],
) -> std::io::Result<()> {
    if let Some((_, cl)) = headers.iter().find(|(k, _)| k == "content-length") {
        let len: u64 = cl.parse().unwrap_or(0);
        let mut remaining = len;
        let mut sink = [0u8; 64 * 1024];
        while remaining > 0 {
            let step = remaining.min(sink.len() as u64) as usize;
            reader.read_exact(&mut sink[..step]).await?;
            remaining -= step as u64;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_tus(
    method: &str,
    target: &str,
    headers: &[(String, String)],
    reader: &mut BufReader<tokio::net::TcpStream>,
    state: &Arc<Mutex<RecorderState>>,
    throttle_bps: Option<u64>,
    record_file: &Option<PathBuf>,
) -> std::io::Result<String> {
    if !tus_version_ok(headers) {
        return Ok(
            "HTTP/1.1 412 Precondition Failed\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n"
                .to_string(),
        );
    }
    // Session id: `/files/<id>`; creation is exactly `/files` (a trailing
    // slash or a glued path like `/filesdoes-not-exist` is an unknown
    // session, and PATCH/HEAD on it must 404 — SessionLost's wire shape).
    let id = if target == "/files" {
        None
    } else {
        let rest = target.strip_prefix("/files").unwrap_or(target);
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        if rest.is_empty() {
            None
        } else {
            Some(rest)
        }
    };
    match (method, id) {
        // ---- creation: POST /files + Upload-Length → 201 + Location -------
        ("POST", None) => {
            let Some(len_raw) = header(headers, "upload-length") else {
                return Ok("HTTP/1.1 400 Bad Request\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string());
            };
            let Ok(length) = len_raw.parse::<u64>() else {
                return Ok("HTTP/1.1 400 Bad Request\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string());
            };
            drain_body(reader, headers).await?;
            let sid = {
                let mut st = state.lock().unwrap();
                st.tus_next += 1;
                let sid = format!("e2e{:08x}", st.tus_next);
                st.tus.insert(
                    sid.clone(),
                    Arc::new(Mutex::new(TusSession {
                        length,
                        buf: Vec::with_capacity(length.min(64 * 1024 * 1024) as usize),
                    })),
                );
                st.tus_last = Some(sid.clone());
                sid
            };
            write_record(record_file, &state.lock().unwrap());
            eprintln!("[loopback-recorder] TUS creation /files/{sid} length={length}");
            // Relative Location on purpose: the adapter must resolve it
            // (RFC 3986 §5.2 merge) — tusd sends absolute, the spec allows
            // both, and the e2e then covers the relative face.
            Ok(format!(
                "HTTP/1.1 201 Created\r\ntus-resumable: 1.0.0\r\nlocation: /files/{sid}\r\ncontent-length: 0\r\n\r\n"
            ))
        }
        // ---- session truth: HEAD /files/<id> → offset + length ------------
        ("HEAD", Some(sid)) => {
            let resp = match state.lock().unwrap().tus.get(sid) {
                None => "HTTP/1.1 404 Not Found\r\ntus-resumable: 1.0.0\r\ncache-control: no-store\r\ncontent-length: 0\r\n\r\n".to_string(),
                Some(s) => {
                    let s = s.lock().unwrap();
                    format!(
                        "HTTP/1.1 200 OK\r\ntus-resumable: 1.0.0\r\nupload-offset: {}\r\nupload-length: {}\r\ncache-control: no-store\r\ncontent-length: 0\r\n\r\n",
                        s.buf.len(),
                        s.length
                    )
                }
            };
            eprintln!("[loopback-recorder] TUS HEAD /files/{sid}");
            Ok(resp)
        }
        // ---- transfer: PATCH /files/<id> → 204 + the acked new offset -----
        ("PATCH", Some(sid)) => {
            let session = {
                let st = state.lock().unwrap();
                st.tus.get(sid).cloned()
            };
            let Some(session) = session else {
                // Unknown session: drain (tusd-like) then 404.
                drain_body(reader, headers).await?;
                return Ok("HTTP/1.1 404 Not Found\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string());
            };
            let Some(off_raw) = header(headers, "upload-offset") else {
                drain_body(reader, headers).await?;
                return Ok("HTTP/1.1 400 Bad Request\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string());
            };
            let Ok(attempted) = off_raw.parse::<u64>() else {
                drain_body(reader, headers).await?;
                return Ok("HTTP/1.1 400 Bad Request\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string());
            };
            let content_type_ok = header(headers, "content-type")
                .map(|v| v.starts_with("application/offset+octet-stream"))
                .unwrap_or(false);
            if !content_type_ok {
                drain_body(reader, headers).await?;
                return Ok("HTTP/1.1 415 Unsupported Media Type\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string());
            }
            let span: u64 = header(headers, "content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            // Offset gate BEFORE consuming the body — the protocol's 409.
            let current = session.lock().unwrap().buf.len() as u64;
            if attempted != current {
                drain_body(reader, headers).await?;
                eprintln!(
                    "[loopback-recorder] TUS PATCH /files/{sid} 409: attempted {attempted}, server at {current}"
                );
                return Ok(
                    "HTTP/1.1 409 Conflict\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n"
                        .to_string(),
                );
            }
            // Throttled consume, then commit: the bytes are "on the server"
            // only when fully read, like tusd's file store on a full read.
            let mut body = Vec::with_capacity(span as usize);
            if let Some(bps) = throttle_bps {
                let mut remaining = span;
                let mut chunk = [0u8; 64 * 1024];
                while remaining > 0 {
                    let step = remaining.min(chunk.len() as u64) as usize;
                    reader.read_exact(&mut chunk[..step]).await?;
                    body.extend_from_slice(&chunk[..step]);
                    remaining -= step as u64;
                    let secs = (step as f64) / (bps as f64);
                    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
                }
            } else {
                let mut remaining = span;
                let mut chunk = [0u8; 64 * 1024];
                while remaining > 0 {
                    let step = remaining.min(chunk.len() as u64) as usize;
                    reader.read_exact(&mut chunk[..step]).await?;
                    body.extend_from_slice(&chunk[..step]);
                    remaining -= step as u64;
                }
            }
            let new_offset = {
                let mut s = session.lock().unwrap();
                s.buf.extend_from_slice(&body);
                s.buf.len() as u64
            };
            write_record(record_file, &state.lock().unwrap());
            eprintln!(
                "[loopback-recorder] TUS PATCH /files/{sid} +{} → offset {new_offset}",
                body.len()
            );
            Ok(format!(
                "HTTP/1.1 204 No Content\r\ntus-resumable: 1.0.0\r\nupload-offset: {new_offset}\r\ncontent-length: 0\r\n\r\n"
            ))
        }
        // ---- discovery (harmless completeness; the adapter never sends it)
        ("OPTIONS", _) => Ok(
            "HTTP/1.1 204 No Content\r\ntus-resumable: 1.0.0\r\ntus-version: 1.0.0\r\ntus-extension: creation\r\ncontent-length: 0\r\n\r\n"
                .to_string(),
        ),
        _ => Ok("HTTP/1.1 405 Method Not Allowed\r\ntus-resumable: 1.0.0\r\ncontent-length: 0\r\n\r\n".to_string()),
    }
}

fn main() {
    let args = parse_args();
    let state = Arc::new(Mutex::new(RecorderState::default()));

    let listener = std::net::TcpListener::bind(("127.0.0.1", args.port))
        .unwrap_or_else(|e| die(&format!("bind 127.0.0.1:{}: {e}", args.port)));
    let addr: SocketAddr = listener.local_addr().unwrap();
    println!("[loopback-recorder] listening on http://{addr}/");
    eprintln!(
        "[loopback-recorder] throttle={:?} record={:?}",
        args.throttle_bps, args.record_file
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let st = Arc::clone(&state);
            let tf = args.record_file.clone();
            let throttle = args.throttle_bps;
            tokio::spawn(handle_conn(stream, st, throttle, tf));
        }
    });
}
