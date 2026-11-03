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
//! `{"put_count": n, "received_bytes": n}` — the e2e reads it to assert on
//! what the server really holds without needing a second protocol.

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
    let json = format!(
        "{{\"put_count\": {}, \"received_bytes\": {}}}\n",
        state.put_count,
        state.received.len()
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
        let method = request_line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        // ---- body ----------------------------------------------------------
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

        // ---- respond -------------------------------------------------------
        let response = match method.as_str() {
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
        };
        reader.get_mut().write_all(response.as_bytes()).await?;
        reader.get_mut().flush().await?;
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
