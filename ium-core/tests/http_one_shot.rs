//! End-to-end proof for the HTTP one-shot adapter against a REAL loopback
//! TCP server — no mocks on the transport path. The server:
//! * parses request heads,
//! * decodes `Transfer-Encoding: chunked` bodies (what reqwest emits for
//!   unknown-length streams — exactly what real servers must handle),
//! * records every byte it receives,
//! * serves configurable behavior: 200+ETag, 429+Retry-After, 500, and a
//!   `HEAD` that reports the number of bytes it actually holds (or lies, for
//!   the verify-gate test).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{ChunkState, HttpOneShotAdapter, JobSpec, SessionJournal};

const MIB: u64 = 1024 * 1024;

#[derive(Clone)]
enum PutBehavior {
    /// Accept with 200 + ETag.
    Accept,
    /// First N PUTs answered with this status (+optional Retry-After seconds),
    /// then accept.
    FailFirst(u32, u16, Option<u64>),
    /// Always fail with this status.
    Always(u16),
    /// Accept PUTs honestly, but HEAD reports a wrong byte count (a server
    /// that lies about what it holds — the verify gate must catch it).
    AcceptButLieOnHead,
}

struct ServerState {
    behavior: Mutex<PutBehavior>,
    /// Bytes actually received across all PUT bodies.
    received: Mutex<Vec<u8>>,
    put_count: Mutex<u32>,
}

struct TestServer {
    addr: SocketAddr,
    state: Arc<ServerState>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl TestServer {
    fn set_behavior(&self, b: PutBehavior) {
        *self.state.behavior.lock().unwrap() = b;
    }
    fn received(&self) -> Vec<u8> {
        self.state.received.lock().unwrap().clone()
    }
    fn put_count(&self) -> u32 {
        *self.state.put_count.lock().unwrap()
    }
}

fn spawn_server() -> TestServer {
    let state = Arc::new(ServerState {
        behavior: Mutex::new(PutBehavior::Accept),
        received: Mutex::new(Vec::new()),
        put_count: Mutex::new(0),
    });
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let runtime_state = Arc::clone(&state);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();

    tokio::spawn(async move {
        let mut shutdown = rx;
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let st = Arc::clone(&runtime_state);
                    tokio::spawn(handle_conn(stream, st));
                }
            }
        }
    });

    TestServer {
        addr,
        state,
        shutdown: tx,
    }
}

async fn handle_conn(
    stream: tokio::net::TcpStream,
    state: Arc<ServerState>,
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
        eprintln!("[srv] {} hdrs={:?}", request_line.trim(), headers);
        let method = request_line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        // ---- body ----------------------------------------------------------
        let body = if headers
            .iter()
            .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"))
        {
            read_chunked(&mut reader).await?
        } else if let Some((_, cl)) = headers.iter().find(|(k, _)| k == "content-length") {
            let len: u64 = cl.parse().unwrap_or(0);
            let mut buf = vec![0u8; len as usize];
            reader.read_exact(&mut buf).await?;
            buf
        } else {
            Vec::new()
        };

        // ---- respond ---------------------------------------------------------
        let response = match method.as_str() {
            "PUT" => {
                eprintln!("[srv] PUT body={} bytes", body.len());
                *state.put_count.lock().unwrap() += 1;
                let behavior = state.behavior.lock().unwrap().clone();
                // Decide the fate of this attempt BEFORE committing bytes:
                // a rejected upload is read off the wire but NOT committed —
                // exactly how real endpoints behave.
                let rejected = match &behavior {
                    PutBehavior::Always(_) => true,
                    PutBehavior::FailFirst(times, ..) => *state.put_count.lock().unwrap() <= *times,
                    _ => false,
                };
                if !rejected {
                    state.received.lock().unwrap().extend_from_slice(&body);
                }
                match behavior {
                    PutBehavior::Accept | PutBehavior::AcceptButLieOnHead => {
                        "HTTP/1.1 200 OK\r\netag: \"ium-test\"\r\ncontent-length: 0\r\n\r\n"
                            .to_string()
                    }
                    PutBehavior::FailFirst(times, status, retry_after) => {
                        let count = *state.put_count.lock().unwrap();
                        if count <= times {
                            let ra = retry_after
                                .map(|s| format!("retry-after: {s}\r\n"))
                                .unwrap_or_default();
                            format!("HTTP/1.1 {status} Failed\r\n{ra}content-length: 0\r\n\r\n")
                        } else {
                            "HTTP/1.1 200 OK\r\netag: \"ium-test\"\r\ncontent-length: 0\r\n\r\n"
                                .to_string()
                        }
                    }
                    PutBehavior::Always(status) => {
                        format!("HTTP/1.1 {status} Failed\r\ncontent-length: 0\r\n\r\n")
                    }
                }
            }
            "HEAD" => {
                let held = state.received.lock().unwrap().len();
                eprintln!("[srv] HEAD held={held}");
                let behavior = state.behavior.lock().unwrap().clone();
                match behavior {
                    PutBehavior::AcceptButLieOnHead => {
                        let reported = held.saturating_sub(1024);
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {reported}\r\n\r\n")
                    }
                    _ => format!("HTTP/1.1 200 OK\r\ncontent-length: {held}\r\n\r\n"),
                }
            }
            _ => "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n".to_string(),
        };
        reader.get_mut().write_all(response.as_bytes()).await?;
        reader.get_mut().flush().await?;
    }
}

async fn read_chunked(reader: &mut BufReader<tokio::net::TcpStream>) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line).await?;
        let size_str = size_line.trim().split(';').next().unwrap_or("0").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if size == 0 {
            // trailers until blank line
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
        let mut crlf = String::new();
        reader.read_line(&mut crlf).await?; // trailing CRLF after each chunk
    }
    Ok(body)
}

fn make_source(dir: &std::path::Path, size: u64) -> (PathBuf, Vec<u8>) {
    let path = dir.join("upload-me.bin");
    let mut bytes = Vec::with_capacity(size as usize);
    for off in 0..size {
        bytes.push((off % 253) as u8);
    }
    std::fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

fn manager_with_http() -> JobManager {
    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    let adapter =
        HttpOneShotAdapter::with_client_config(Duration::from_secs(3), Duration::from_secs(20))
            .expect("http client builds");
    m.register_adapter(Arc::new(adapter));
    m
}

fn spec_for(path: &std::path::Path, size: u64, addr: SocketAddr) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "http_oneshot".into(),
        endpoint: format!("http://{addr}/upload"),
    }
}

#[tokio::test]
async fn http_one_shot_streams_whole_file_and_verifies() {
    let server = spawn_server();
    let dir = tempfile::tempdir().unwrap();
    let (path, file_bytes) = make_source(dir.path(), 2 * MIB);

    let m = manager_with_http();
    let rec = m
        .create_job(&spec_for(&path, file_bytes.len() as u64, server.addr), 4)
        .unwrap();

    // one-shot → single chunk covering the whole file, honest badge
    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(snap.chunks.len(), 1);
    assert_eq!(snap.job.capability_badge, "one_shot");

    let done = m.run_job(&rec.id).await.unwrap();
    if done.state != JobState::Done {
        for (seq, k, p) in m.snapshot(&rec.id).unwrap().events {
            eprintln!("EVENT {seq:>3} {k:<18} {p}");
        }
    }
    assert_eq!(done.state, JobState::Done, "expected success: {done:?}");

    // The server must have received the EXACT file bytes, exactly once.
    assert_eq!(server.put_count(), 1);
    assert_eq!(server.received(), file_bytes, "server got corrupted bytes");

    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(snap.chunks[0].state, ChunkState::Verified);
    // ETag captured as the adapter token (reattach evidence)
    assert_eq!(
        snap.chunks[0].adapter_token.as_deref(),
        Some("\"ium-test\"")
    );
    // verify-before-declare ran and named the byte count
    assert_eq!(
        snap.events
            .iter()
            .filter(|(_, k, _)| k == "verified")
            .count(),
        1
    );
    assert!(snap
        .events
        .iter()
        .any(|(_, k, p)| k == "verified" && p.contains("server holds all")));
    server.shutdown.send(()).ok();
}

#[tokio::test]
async fn http_500_exhausts_retries_and_fails_honestly() {
    let server = spawn_server();
    server.set_behavior(PutBehavior::Always(500));

    let dir = tempfile::tempdir().unwrap();
    let (path, _) = make_source(dir.path(), 256 * 1024);

    let mut m = manager_with_http();
    m.set_policy(ium_core::retry::RetryPolicy {
        max_attempts: 2,
        backoff_base_ms: 1,
        backoff_cap_ms: 2,
        respect_retry_after: true,
    });

    let rec = m
        .create_job(&spec_for(&path, 256 * 1024, server.addr), 4)
        .unwrap();
    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(out.state, JobState::Failed);

    // 1 initial + 2 retries = 3 PUTs, each failing with 500
    assert_eq!(server.put_count(), 3);
    let snap = m.snapshot(&rec.id).unwrap();
    assert!(snap
        .events
        .iter()
        .any(|(_, k, p)| k == "chunk_error" && p.contains("Transient")));
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        0,
        "a failed one-shot job must keep ZERO claimed bytes"
    );
    server.shutdown.send(()).ok();
}

#[tokio::test]
async fn http_429_with_retry_after_recovers() {
    let server = spawn_server();
    server.set_behavior(PutBehavior::FailFirst(1, 429, Some(0)));

    let dir = tempfile::tempdir().unwrap();
    let (path, file_bytes) = make_source(dir.path(), 128 * 1024);

    let m = manager_with_http();
    let rec = m
        .create_job(&spec_for(&path, file_bytes.len() as u64, server.addr), 4)
        .unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    if done.state != JobState::Done {
        for (seq, k, p) in m.snapshot(&rec.id).unwrap().events {
            eprintln!("EVENT {seq:>3} {k:<18} {p}");
        }
    }
    assert_eq!(done.state, JobState::Done);

    assert_eq!(server.put_count(), 2, "throttled once, retried once");
    assert_eq!(server.received(), file_bytes);
    let snap = m.snapshot(&rec.id).unwrap();
    assert!(snap
        .events
        .iter()
        .any(|(_, k, p)| k == "chunk_error" && p.contains("Throttled")));
    server.shutdown.send(()).ok();
}

#[tokio::test]
async fn verify_gate_catches_a_server_that_lies_about_bytes() {
    // The PUT succeeds, but the server silently dropped bytes: HEAD reports
    // less than we sent → the verify gate must refuse to declare Done.
    let server = spawn_server();
    server.set_behavior(PutBehavior::AcceptButLieOnHead);

    let dir = tempfile::tempdir().unwrap();
    let (path, file_bytes) = make_source(dir.path(), 64 * 1024);

    let m = manager_with_http();
    let rec = m
        .create_job(&spec_for(&path, file_bytes.len() as u64, server.addr), 4)
        .unwrap();

    let out = m.run_job(&rec.id).await.unwrap();
    assert_eq!(
        out.state,
        JobState::Failed,
        "a server that lost bytes must not be declared Done"
    );
    let snap = m.snapshot(&rec.id).unwrap();
    assert!(
        snap.events
            .iter()
            .any(|(_, k, p)| k == "verify_failed" && p.contains("server holds")),
        "the verify failure detail must name the byte discrepancy"
    );
    server.shutdown.send(()).ok();
}
