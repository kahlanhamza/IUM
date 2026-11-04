//! A scriptable, tus-shaped loopback server for the slow-lane (#42) and
//! own-drift (#43) drills. Wire facts follow the tus 1.0.0 core the adapter
//! already pins (see `adapter_tus` module docs): creation POST → 201 +
//! Location; PATCH + Upload-Offset → 204 + Upload-Offset; HEAD → 200 +
//! Upload-Offset/Upload-Length. This mock adds the impairments the real
//! servers cannot be asked to perform on demand:
//!
//! * `KillAfterReading` — persist N received bytes, then drop the connection
//!   WITHOUT a response: the client-side death that leaves the server AHEAD
//!   of the journal (the own-drift #43 shape; tusd persists what arrived
//!   before the socket died, captured on the live rehearsal #33/#37).
//! * `Stall` — stop reading and never respond, holding the connection open:
//!   a dead tunnel with a full kernel buffer (the #42 idle-progress shape).
//! * `Pace` — read the body at a fixed B/s: a steady slow uplink with real
//!   socket backpressure (not a simulated delay before an instant body).
//!
//! All state lives behind atomics/mutexes; every connection is its own task,
//! so a killed PATCH connection never blocks the HEAD that classifies it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What the server does with one PATCH connection's body.
#[derive(Debug, Clone)]
pub enum PatchBehavior {
    /// Persist `bytes` received bytes, then close WITHOUT any response —
    /// the client observes a transport death; the bytes it read are durable
    /// (server offset advances). Exactly the timed-out-PATCH aftermath.
    KillAfterReading { bytes: u64 },
    /// Hold the connection open, read NOTHING, respond with NOTHING — the
    /// stalled-tunnel shape (client's kernel buffers fill, then silence).
    Stall,
    /// Read the whole body at `bps` bytes/second, persist it, answer a
    /// normal 204 — a steady slow uplink (the owner's 3 KB/s moment).
    Pace { bps: f64 },
    /// Read the whole body instantly, persist, answer 204 — a healthy link.
    Accept,
}

pub struct TusMock {
    pub addr: std::net::SocketAddr,
    state: Arc<MockState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

struct MockState {
    store: Mutex<Vec<u8>>,
    offset: AtomicU64,
    length: AtomicU64,
    script: Mutex<VecDeque<PatchBehavior>>,
    /// Global read pace (bytes/sec) applied to EVERY PATCH body read — the
    /// whole uplink is slow, not one scripted connection. `None` = full
    /// speed. This is the owner's-daily-reality shape (#18): no victims,
    /// a uniformly slow wire.
    read_pace_bps: Mutex<Option<f64>>,
    /// tusd wire fact: PATCHes on ONE upload resource are serialized — a
    /// second concurrent PATCH is answered 423 Locked. Without this, an
    /// abandoned paced connection still draining its receive buffer races
    /// the next PATCH's writes at the shared offset and corrupts the store.
    patch_busy: AtomicBool,
    /// Diagnostics for assertions.
    patches_killed: AtomicU64,
    patches_completed: AtomicU64,
}

/// Holds `patch_busy` for one PATCH connection; released on every exit
/// path (normal 204, mid-body kill, client abort, stall end).
struct PatchGuard<'a>(&'a AtomicBool);
impl Drop for PatchGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl TusMock {
    /// Spawn with a PATCH script consumed in order; when the script is
    /// exhausted every further PATCH is `Accept` (a recovered link).
    pub fn spawn(script: Vec<PatchBehavior>, length: u64) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(MockState {
            store: Mutex::new(Vec::new()),
            offset: AtomicU64::new(0),
            length: AtomicU64::new(length),
            script: Mutex::new(script.into()),
            read_pace_bps: Mutex::new(None),
            patch_busy: AtomicBool::new(false),
            patches_killed: AtomicU64::new(0),
            patches_completed: AtomicU64::new(0),
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let st = state.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("mock runtime");
            runtime.block_on(async move {
                listener
                    .set_nonblocking(true)
                    .expect("nonblocking listener");
                let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                let mut shutdown = rx;
                loop {
                    tokio::select! {
                        _ = &mut shutdown => break,
                        accepted = listener.accept() => {
                            let Ok((stream, _)) = accepted else { continue };
                            let st = st.clone();
                            tokio::spawn(async move { handle(stream, st).await });
                        }
                    }
                }
            });
        });
        TusMock {
            addr,
            state,
            shutdown: Some(tx),
        }
    }

    pub fn endpoint(&self) -> String {
        format!("http://{}/files/", self.addr)
    }

    pub fn offset(&self) -> u64 {
        self.state.offset.load(Ordering::SeqCst)
    }

    pub fn stored(&self) -> Vec<u8> {
        self.state.store.lock().unwrap().clone()
    }

    pub fn patches_killed(&self) -> u64 {
        self.state.patches_killed.load(Ordering::SeqCst)
    }

    pub fn patches_completed(&self) -> u64 {
        self.state.patches_completed.load(Ordering::SeqCst)
    }

    /// Pace EVERY PATCH body read at `bps` bytes/second (None = full
    /// speed) — the uniformly-slow-uplink shape (#18), independent of the
    /// per-PATCH script.
    pub fn set_read_pace_bps(&self, bps: Option<f64>) {
        *self.state.read_pace_bps.lock().unwrap() = bps;
    }
}

impl Drop for TusMock {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

struct Request {
    method: String,
    content_length: u64,
    upload_offset: Option<u64>,
}

/// Read one request head (through the blank line). Returns None on EOF.
async fn read_head(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        // 10 s head budget: a drill that cannot even deliver headers is a
        // broken drill, and a hanging one poisons CI worse than a panic.
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut byte))
            .await
            .ok()?
            .expect("read head");
        if n == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            panic!("request head exceeded 64 KiB — parser bug");
        }
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let method = request_line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let content_length = head
        .lines()
        .find_map(|l| {
            // hyper writes header names LOWERCASE on the wire (the http
            // crate stores them normalized) — match case-insensitively.
            let (name, value) = l.split_once(':')?;
            if !name.eq_ignore_ascii_case("Content-Length") {
                return None;
            }
            value.trim().parse::<u64>().ok()
        })
        .unwrap_or(0);
    let upload_offset = head.lines().find_map(|l| {
        let (name, value) = l.split_once(':')?;
        if !name.eq_ignore_ascii_case("Upload-Offset") {
            return None;
        }
        value.trim().parse::<u64>().ok()
    });
    Some(Request {
        method,
        content_length,
        upload_offset,
    })
}

async fn respond(stream: &mut TcpStream, status: &str, extra: &[(&str, String)]) {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    head.push_str("Connection: close\r\n");
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("Content-Length: 0\r\n\r\n");
    stream
        .write_all(head.as_bytes())
        .await
        .expect("write response");
    stream.flush().await.expect("flush response");
}

async fn persist(state: &MockState, data: Vec<u8>) {
    let mut store = state.store.lock().unwrap();
    let base = state.offset.load(Ordering::SeqCst) as usize;
    // Store is written at the server offset (realign re-PATCHes start there).
    if store.len() < base + data.len() {
        store.resize(base + data.len(), 0);
    }
    store[base..base + data.len()].copy_from_slice(&data);
    drop(store);
    state.offset.fetch_add(data.len() as u64, Ordering::SeqCst);
    drop(data);
}

async fn handle(mut stream: TcpStream, state: Arc<MockState>) {
    let Some(req) = read_head(&mut stream).await else {
        return;
    };
    match req.method.as_str() {
        "POST" => {
            // Creation: 201 + absolute Location (tusd shape).
            let location = format!(
                "http://{}/files/ium-mock-1",
                stream.local_addr().expect("local")
            );
            respond(
                &mut stream,
                "201 Created",
                &[("Location", location), ("Tus-Resumable", "1.0.0".into())],
            )
            .await;
        }
        "HEAD" => {
            respond(
                &mut stream,
                "200 OK",
                &[
                    (
                        "Upload-Offset",
                        state.offset.load(Ordering::SeqCst).to_string(),
                    ),
                    (
                        "Upload-Length",
                        state.length.load(Ordering::SeqCst).to_string(),
                    ),
                    ("Tus-Resumable", "1.0.0".into()),
                    ("Cache-Control", "no-store".into()),
                ],
            )
            .await;
        }
        "PATCH" => {
            // tusd wire fact: a PATCH whose Upload-Offset ≠ the server's
            // current offset is rejected with 409 AT THE REQUEST HEAD — the
            // body is not consumed. Without this, a journal-behind-server
            // retry would silently re-append bytes the server already
            // holds, corrupting the store instead of exercising the
            // adapter's realign dance.
            if req.upload_offset != Some(state.offset.load(Ordering::SeqCst)) {
                respond(
                    &mut stream,
                    "409 Conflict",
                    &[
                        (
                            "Upload-Offset",
                            state.offset.load(Ordering::SeqCst).to_string(),
                        ),
                        ("Tus-Resumable", "1.0.0".into()),
                    ],
                )
                .await;
                return;
            }
            // One writer per upload resource (tusd wire fact): a concurrent
            // PATCH — e.g. one arriving while an abandoned paced connection
            // is still draining — gets 423 Locked, and the adapter's retry
            // ladder absorbs it (423 → Transient).
            if state
                .patch_busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                respond(
                    &mut stream,
                    "423 Locked",
                    &[("Tus-Resumable", "1.0.0".into())],
                )
                .await;
                return;
            }
            let _guard = PatchGuard(&state.patch_busy);
            let behavior = state
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(PatchBehavior::Accept);
            match behavior {
                PatchBehavior::Stall => {
                    // Hold the connection, reading nothing useful: the
                    // client's watchdog must be the one to cut this. The
                    // stall ENDS when the client aborts (RST/EOF reaches the
                    // socket — a real dead peer does the same) or after 120 s
                    // — either way the per-upload lock releases with it.
                    let mut sink = [0u8; 512];
                    let _ = tokio::time::timeout(Duration::from_secs(120), stream.read(&mut sink))
                        .await;
                }
                PatchBehavior::KillAfterReading { bytes } => {
                    let mut taken = 0u64;
                    let mut buf = vec![0u8; 16 * 1024];
                    while taken < bytes {
                        let want = (bytes - taken).min(buf.len() as u64) as usize;
                        // The client may legitimately stop sending first
                        // (span smaller than the kill budget): a short read
                        // with the body exhausted ends the attempt.
                        let read = match tokio::time::timeout(
                            Duration::from_secs(5),
                            stream.read(&mut buf[..want]),
                        )
                        .await
                        {
                            Ok(Ok(n)) if n > 0 => n,
                            _ => break,
                        };
                        persist(&state, buf[..read].to_vec()).await;
                        taken += read as u64;
                    }
                    state.patches_killed.fetch_add(1, Ordering::SeqCst);
                    // Drop WITHOUT a response — the connection dies mid-flight.
                    let _ = stream.shutdown().await;
                }
                PatchBehavior::Pace { bps } => {
                    let mut remaining = req.content_length;
                    let mut buf = vec![0u8; 8 * 1024];
                    while remaining > 0 {
                        let want = remaining.min(buf.len() as u64) as usize;
                        let read = match tokio::time::timeout(
                            Duration::from_secs(30),
                            stream.read(&mut buf[..want]),
                        )
                        .await
                        {
                            Ok(Ok(n)) if n > 0 => n,
                            _ => break,
                        };
                        persist(&state, buf[..read].to_vec()).await;
                        remaining -= read as u64;
                        // THE UPLINK: this sleep is the slow wire.
                        tokio::time::sleep(Duration::from_secs_f64(read as f64 / bps)).await;
                    }
                    state.patches_completed.fetch_add(1, Ordering::SeqCst);
                    respond(
                        &mut stream,
                        "204 No Content",
                        &[
                            (
                                "Upload-Offset",
                                state.offset.load(Ordering::SeqCst).to_string(),
                            ),
                            ("Tus-Resumable", "1.0.0".into()),
                        ],
                    )
                    .await;
                }
                PatchBehavior::Accept => {
                    let pace = state.read_pace_bps.lock().unwrap().to_owned();
                    let mut remaining = req.content_length;
                    let mut buf = vec![0u8; 64 * 1024];
                    while remaining > 0 {
                        let want = remaining.min(buf.len() as u64) as usize;
                        let read = match tokio::time::timeout(
                            Duration::from_secs(30),
                            stream.read(&mut buf[..want]),
                        )
                        .await
                        {
                            Ok(Ok(n)) if n > 0 => n,
                            _ => break,
                        };
                        persist(&state, buf[..read].to_vec()).await;
                        remaining -= read as u64;
                        // THE UPLINK (global pace): this sleep is the slow wire.
                        if let Some(bps) = pace {
                            tokio::time::sleep(Duration::from_secs_f64(read as f64 / bps)).await;
                        }
                    }
                    state.patches_completed.fetch_add(1, Ordering::SeqCst);
                    respond(
                        &mut stream,
                        "204 No Content",
                        &[
                            (
                                "Upload-Offset",
                                state.offset.load(Ordering::SeqCst).to_string(),
                            ),
                            ("Tus-Resumable", "1.0.0".into()),
                        ],
                    )
                    .await;
                }
            }
        }
        _ => {
            respond(&mut stream, "404 Not Found", &[]).await;
        }
    }
}
