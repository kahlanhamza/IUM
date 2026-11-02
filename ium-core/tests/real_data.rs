//! Real-data e2e drill (issue #17 — owner mandate 2026-11-02: always test on
//! real data). Every other suite in this repo uploads zeros or a fixed
//! pattern; this file pushes a REAL, playable H.264/AAC video through the
//! ENGINE (`create_job` → `run_job` → honest Done from the journal) and
//! proves byte fidelity end to end:
//!
//! * fixture: generated at test time by ffmpeg (`testsrc2` + sine audio,
//!   ~23 MiB) — never committed (ADR-009: binaries are not repo evidence);
//!   skipped LOUDLY where ffmpeg/ffprobe are absent, same honesty contract
//!   as the env-gated live suites.
//! * reality guards (a zeros file cannot pass): ffprobe must report an
//!   `h264` video stream with `duration > 0` plus an audio stream, and a
//!   64 KiB sample window must hold > 64 distinct byte values.
//! * fidelity invariant: `sha256(received) == sha256(source as generated)`
//!   — a pinned hash would be wrong (ffmpeg output differs per version);
//!   the entropy + codec guards make "source" real in every environment.
//!
//! Lanes:
//! 1. HTTP loopback recorder server — always runs where ffmpeg exists.
//! 2. TUS vs pinned tusd — env-gated `IUM_TUS_ENDPOINT` (scripts/tus-test.sh).
//! 3. S3 multipart vs pinned SeaweedFS — env-gated `IUM_S3_ENDPOINT`
//!    (scripts/s3-test.sh); real content makes the parts reassembly
//!    meaningful (arbitrary-order parts must reassemble into a PLAYABLE file).

use std::io::{Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{
    HttpOneShotAdapter, JobSpec, S3MultipartAdapter, S3MultipartConfig, SessionJournal, TusAdapter,
};

const SAMPLE_WINDOW: usize = 64 * 1024;
const DISTINCT_BYTES_REQUIRED: usize = 64;

// ---------------------------------------------------------------------------
// fixture — a REAL video, generated, guarded, never committed
// ---------------------------------------------------------------------------

struct Fixture {
    path: PathBuf,
    size: u64,
    sha256: String,
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generate the real fixture; `None` = loud skip (no ffmpeg/ffprobe).
fn real_video(dir: &Path) -> Option<Fixture> {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!(
            "skipped: ffmpeg/ffprobe not on PATH — the real-data drill refuses to \
             fake it with zeros (owner mandate: always test on real data)"
        );
        return None;
    }
    let out = dir.join("ium-real-fixture.mp4");
    let st = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
            "-t",
            "45",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "4M",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-shortest",
            "-movflags",
            "+faststart",
        ])
        .arg(&out)
        .output()
        .expect("spawn ffmpeg");
    assert!(
        st.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
    let size = out.metadata().expect("fixture stat").len();
    assert!(
        size > 16 * 1024 * 1024,
        "fixture too small to be multi-chunk: {size}"
    );
    assert_real_video(&out, "source");
    assert_not_trivial(&out, size, "source");
    let sha256 = sha256_file(&out);
    Some(Fixture {
        path: out,
        size,
        sha256,
    })
}

/// ffprobe guards: a REAL video (h264 + aac, duration > 0) — pins that the
/// drill can never silently degrade to zeros or random bytes.
fn assert_real_video(path: &Path, label: &str) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format"])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(
        out.status.success(),
        "ffprobe failed on {label}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("codec_name=h264") && text.contains("codec_type=video"),
        "{label} is not a real H.264 video (mandate violation)"
    );
    assert!(
        text.contains("codec_name=aac") && text.contains("codec_type=audio"),
        "{label} lost its audio stream"
    );
    let duration = text
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0);
    assert!(
        duration > 0.0,
        "{label} has no duration — not a playable video"
    );
}

/// Entropy guard: a 64 KiB window around the middle must hold > 64 distinct
/// byte values. Zeros (or any single-byte pattern) hold exactly 1 — this is
/// the cheap, std-only tripwire that keeps the drill honest.
fn assert_not_trivial(path: &Path, size: u64, label: &str) {
    let mut f = std::fs::File::open(path).expect("open for entropy check");
    let window = SAMPLE_WINDOW.min(size as usize);
    let mid = (size / 2).saturating_sub(window as u64 / 2);
    f.seek(std::io::SeekFrom::Start(mid)).expect("seek");
    let mut buf = vec![0u8; window];
    std::io::Read::read_exact(&mut f, &mut buf).expect("read entropy window");
    let distinct = buf.iter().collect::<std::collections::HashSet<_>>().len();
    assert!(
        distinct > DISTINCT_BYTES_REQUIRED,
        "{label} looks trivial ({distinct} distinct byte values in a {window}-byte window) — \
         the real-data drill only accepts real content"
    );
}

fn sha256_file(path: &Path) -> String {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(path).expect("open for sha256");
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).expect("read for sha256");
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    hex(h.finalize())
}

fn sha256_bytes(b: &[u8]) -> String {
    hex(Sha256::digest(b))
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Persist received bytes and run the SAME reality guards on them — the
/// destination must hold a playable video, not just the right byte count.
fn assert_received_is_real_video(bytes: &[u8], dir: &Path, label: &str) {
    let p = dir.join(format!("{label}-received.mp4"));
    let mut f = std::fs::File::create(&p).expect("write received");
    f.write_all(bytes).expect("write received");
    f.sync_all().expect("sync received");
    assert_real_video(&p, label);
    assert_not_trivial(&p, bytes.len() as u64, label);
}

// ---------------------------------------------------------------------------
// HTTP lane: recorder loopback server (PUT + honest HEAD)
// ---------------------------------------------------------------------------

struct Recorder {
    addr: std::net::SocketAddr,
    received: Arc<std::sync::Mutex<Vec<u8>>>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

fn spawn_recorder() -> Recorder {
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let rt_received = Arc::clone(&received);
    tokio::spawn(async move {
        let mut shutdown = rx;
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let st = Arc::clone(&rt_received);
                    tokio::spawn(handle(stream, st));
                }
            }
        }
    });
    Recorder {
        addr,
        received,
        shutdown: tx,
    }
}

async fn handle(
    stream: tokio::net::TcpStream,
    received: Arc<std::sync::Mutex<Vec<u8>>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).await? == 0 {
            return Ok(());
        }
        let mut headers: Vec<(String, String)> = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 {
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

        if method == "HEAD" {
            let held = received.lock().unwrap().len();
            let resp = format!("HTTP/1.1 200 OK\r\ncontent-length: {held}\r\n\r\n");
            reader.get_mut().write_all(resp.as_bytes()).await?;
            reader.get_mut().flush().await?;
            continue;
        }
        if method != "PUT" {
            let resp = "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n";
            reader.get_mut().write_all(resp.as_bytes()).await?;
            reader.get_mut().flush().await?;
            continue;
        }

        // The adapter streams real files; reqwest may emit chunked encoding.
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
        received.lock().unwrap().extend_from_slice(&body);
        let resp = "HTTP/1.1 200 OK\r\netag: \"ium-real-data\"\r\ncontent-length: 0\r\n\r\n";
        reader.get_mut().write_all(resp.as_bytes()).await?;
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
        reader.read_line(&mut crlf).await?;
    }
    Ok(body)
}

// ---------------------------------------------------------------------------
// lane 1 — HTTP loopback: the whole real video through the engine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_real_video_through_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let Some(fx) = real_video(dir.path()) else {
        return;
    };
    let server = spawn_recorder();

    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(
        HttpOneShotAdapter::with_client_config(Duration::from_secs(5), Duration::from_secs(60))
            .expect("http client builds"),
    ));

    let spec = JobSpec {
        source_path: fx.path.to_string_lossy().into_owned(),
        source_size: fx.size,
        adapter: "http_oneshot".into(),
        endpoint: format!("http://{}/upload", server.addr),
    };
    let rec = m.create_job(&spec, 4).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(
        done.state,
        JobState::Done,
        "engine must declare Done honestly"
    );

    let received = server.received.lock().unwrap().clone();
    let _ = server.shutdown.send(());
    assert_eq!(
        received.len() as u64,
        fx.size,
        "server must hold exactly the video's byte count"
    );
    assert_eq!(
        sha256_bytes(&received),
        fx.sha256,
        "received bytes are NOT the video — fidelity broken"
    );
    assert_received_is_real_video(&received, dir.path(), "http");
    println!(
        "[real-data/http] OK: {} bytes, sha256 {} — a real H.264/AAC video went through the engine",
        fx.size, fx.sha256
    );
}

// ---------------------------------------------------------------------------
// lane 2 — TUS vs pinned tusd (env-gated)
// ---------------------------------------------------------------------------

async fn tus_read_back(session: &str) -> Vec<u8> {
    let resp = reqwest::get(session.to_string())
        .await
        .unwrap_or_else(|e| panic!("GET read-back from {session}: {e}"));
    assert_eq!(resp.status().as_u16(), 200, "read-back GET from {session}");
    resp.bytes().await.unwrap().to_vec()
}

fn journaled_session_token(db: &Path, job_id: &str) -> String {
    let j = SessionJournal::open(db).expect("reopen journal (WAL, second connection)");
    for (_, kind, payload) in j.events_for(job_id).expect("journal events") {
        if kind == "probe" {
            let v: serde_json::Value = serde_json::from_str(&payload).expect("probe payload json");
            let token = v["session_token"]
                .as_str()
                .expect("probe event carries session_token (ADR-002)")
                .to_string();
            assert!(
                token.starts_with("http"),
                "TUS session token is the creation URL"
            );
            return token;
        }
    }
    panic!("no journaled probe event — the engine never probed");
}

#[tokio::test]
async fn tus_real_video_through_the_engine() {
    let endpoint = match std::env::var("IUM_TUS_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipped: IUM_TUS_ENDPOINT not set (scripts/tus-test.sh boots tusd)");
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(fx) = real_video(dir.path()) else {
        return;
    };
    let db = dir.path().join("journal.db");
    let journal = SessionJournal::open(&db).unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::new(TusAdapter::new()));

    let spec = JobSpec {
        source_path: fx.path.to_string_lossy().into_owned(),
        source_size: fx.size,
        adapter: "tus".into(),
        endpoint,
    };
    let rec = m.create_job(&spec, 4).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(
        done.state,
        JobState::Done,
        "honest Done only after HEAD verify"
    );

    let session = journaled_session_token(&db, &rec.id);
    let received = tus_read_back(&session).await;
    assert_eq!(
        received.len() as u64,
        fx.size,
        "tusd holds the wrong byte count"
    );
    assert_eq!(
        sha256_bytes(&received),
        fx.sha256,
        "received bytes are NOT the video — fidelity broken"
    );
    assert_received_is_real_video(&received, dir.path(), "tus");
    println!(
        "[real-data/tus] OK: {} bytes, sha256 {} — a real H.264/AAC video went through the engine",
        fx.size, fx.sha256
    );
}

// ---------------------------------------------------------------------------
// lane 3 — S3 multipart vs pinned SeaweedFS (env-gated)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn s3_real_video_through_the_engine() {
    let endpoint = match std::env::var("IUM_S3_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipped: IUM_S3_ENDPOINT not set (scripts/s3-test.sh boots SeaweedFS)");
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(fx) = real_video(dir.path()) else {
        return;
    };
    let config = S3MultipartConfig::from_env()
        .unwrap_or_else(|e| panic!("IUM_S3_ENDPOINT is set but creds are not: {e}"));
    let adapter = Arc::new(S3MultipartAdapter::new(config).expect("adapter builds"));

    let journal = SessionJournal::open_in_memory().unwrap();
    let mut m = JobManager::with_journal(journal).unwrap();
    m.register_adapter(Arc::clone(&adapter) as Arc<dyn ium_core::adapter::TransferAdapter>);

    let spec = JobSpec {
        source_path: fx.path.to_string_lossy().into_owned(),
        source_size: fx.size,
        adapter: "s3_multipart".into(),
        endpoint,
    };
    let rec = m.create_job(&spec, 4).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(
        done.state,
        JobState::Done,
        "Done only after ADR-010 verify (ListParts reconcile + HEAD)"
    );

    let received = adapter
        .test_get_object(&spec)
        .await
        .expect("signed GET read-back");
    assert_eq!(
        received.len() as u64,
        fx.size,
        "object holds the wrong byte count"
    );
    assert_eq!(
        sha256_bytes(&received),
        fx.sha256,
        "reassembled object is NOT the video — multipart fidelity broken"
    );
    assert_received_is_real_video(&received, dir.path(), "s3");
    println!(
        "[real-data/s3] OK: {} bytes, sha256 {} — a real H.264/AAC video reassembled from parts",
        fx.size, fx.sha256
    );
}
