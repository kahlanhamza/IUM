//! A scriptable, tus-shaped loopback server for the slow-lane (#42),
//! own-drift (#43), lane-floor (#44), ladder (#45) and prober (#41)
//! drills. Wire facts follow the tus 1.0.0 core the adapter already pins
//! (see `adapter_tus` module docs): creation POST answers 201 plus a
//! Location; PATCH with an Upload-Offset answers 204 plus the new
//! Upload-Offset; HEAD answers 200 with Upload-Offset/Upload-Length.
//! This mock adds the impairments real servers cannot be asked to
//! perform on demand:
//!
//! * `KillAfterReading` — persist N received bytes, then drop the
//!   connection WITHOUT a response: the client-side death that leaves the
//!   server AHEAD of the journal (the own-drift #43 shape; tusd persists
//!   what arrived before the socket died, captured on the live rehearsal
//!   #33/#37).
//! * `Stall` — stop reading and never respond, holding the connection
//!   open: a dead tunnel with a full kernel buffer (the #42 shape).
//! * `Pace` — read the body at a fixed B/s: a steady slow uplink.
//! * a GLOBAL per-connection pace and a GLOBAL shared-bearer pace — the
//!   two link shapes the #41 prober must tell apart.
//!
//! tusd-honesty the drills depend on:
//! * a PATCH whose Upload-Offset ≠ the session's offset is rejected 409
//!   AT THE REQUEST HEAD (the body is not consumed — a naive
//!   always-accept mock would silently re-append bytes and corrupt the
//!   store instead of exercising the realign dance);
//! * ONE writer per session — a concurrent PATCH on the same resource is
//!   answered 423 Locked (real tusd behavior);
//! * creations mint UNIQUE sessions (the #41 prober's K-stream phase runs
//!   K independent sessions concurrently).
//!
//! All state is behind atomics/mutexes; every connection is its own task.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What the server does with one PATCH connection's body.
#[derive(Debug, Clone)]
pub enum PatchBehavior {
    /// Persist `bytes` received bytes, then close WITHOUT any response —
    /// the client observes a transport death; the bytes it read are
    /// durable (session offset advances). The timed-out-PATCH aftermath.
    KillAfterReading { bytes: u64 },
    /// Hold the connection open, read NOTHING, respond with NOTHING —
    /// the stalled-tunnel shape (client's kernel buffers fill, then
    /// silence). Ends when the client aborts (a real dead peer does the
    /// same) or after 120 s.
    Stall,
    /// Read the whole body at `bps` bytes/second, persist it, answer a
    /// normal 204 — a steady slow uplink (the owner's 3 KB/s moment).
    Pace { bps: f64 },
    /// Read the whole body (at the global/bearer pace, if set), persist,
    /// answer 204 — a healthy link.
    Accept,
}

/// One created upload resource: its own durable store and offset.
struct Session {
    store: Mutex<Vec<u8>>,
    offset: AtomicU64,
    length: u64,
    /// ONE writer per session (tusd wire fact) — see MockState docs.
    patch_busy: AtomicBool,
}

struct MockState {
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
    next_id: AtomicU64,
    script: Mutex<VecDeque<PatchBehavior>>,
    /// Per-CONNECTION read pace: N concurrent readers each get the full
    /// `bps` (models per-flow caps; aggregate N×bps).
    read_pace_bps: Mutex<Option<f64>>,
    /// Shared-bearer pace: the AGGREGATE across all concurrent readers
    /// never exceeds `bps` (models the owner's tunnel; the prober's
    /// bearer-cap shape).
    bearer_pace_bps: Mutex<Option<f64>>,
    /// The next free slot on the bearer's timeline.
    bearer_slot: Mutex<tokio::time::Instant>,
    /// Every byte READ off the wire (request heads + bodies, upload
    /// direction) — the #45 ladder's overhead denominator.
    wire_bytes: AtomicU64,
    patches_killed: AtomicU64,
    patches_completed: AtomicU64,
}

pub struct TusMock {
    pub addr: std::net::SocketAddr,
    state: Arc<MockState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TusMock {
    /// Spawn with a PATCH script consumed in order (per session? no — the
    /// script is GLOBAL and consumed in arrival order, matching the
    /// engine's sequential single-worker lane); when exhausted, every
    /// further PATCH is `Accept`.
    pub fn spawn(script: Vec<PatchBehavior>, length: u64) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(MockState {
            sessions: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(0),
            script: Mutex::new(script.into()),
            read_pace_bps: Mutex::new(None),
            bearer_pace_bps: Mutex::new(None),
            bearer_slot: Mutex::new(tokio::time::Instant::now()),
            wire_bytes: AtomicU64::new(0),
            patches_killed: AtomicU64::new(0),
            patches_completed: AtomicU64::new(0),
        });
        // Pre-mint the first session so single-session drills (HEAD before
        // any creation the test makes itself) still see a resource at the
        // documented path.
        state.sessions.lock().unwrap().insert(
            "/files/ium-mock-0".to_string(),
            Arc::new(Session {
                store: Mutex::new(Vec::new()),
                offset: AtomicU64::new(0),
                length,
                patch_busy: AtomicBool::new(false),
            }),
        );
        state.next_id.store(1, Ordering::SeqCst);
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

    /// The ACTIVE session's offset (the one with the most persisted
    /// bytes) — the single-session drills' view. With per-session state
    /// the engine's session is the active one; the pre-minted session
    /// stays empty.
    pub fn offset(&self) -> u64 {
        self.state
            .sessions
            .lock()
            .unwrap()
            .values()
            .map(|s| s.offset.load(Ordering::SeqCst))
            .max()
            .unwrap_or(0)
    }

    /// The ACTIVE session's store (same rule as `offset`).
    pub fn stored(&self) -> Vec<u8> {
        let sessions = self.state.sessions.lock().unwrap();
        let best = sessions
            .values()
            .max_by_key(|s| s.offset.load(Ordering::SeqCst));
        best.map(|s| s.store.lock().unwrap().clone())
            .unwrap_or_default()
    }

    pub fn patches_killed(&self) -> u64 {
        self.state.patches_killed.load(Ordering::SeqCst)
    }

    pub fn patches_completed(&self) -> u64 {
        self.state.patches_completed.load(Ordering::SeqCst)
    }

    /// Total bytes read off the wire (request heads + bodies, upload
    /// direction) — the ladder's overhead measurement denominator.
    pub fn wire_bytes(&self) -> u64 {
        self.state.wire_bytes.load(Ordering::SeqCst)
    }

    /// Per-CONNECTION pace (see MockState.read_pace_bps).
    pub fn set_read_pace_bps(&self, bps: Option<f64>) {
        *self.state.read_pace_bps.lock().unwrap() = bps;
    }

    /// Shared-bearer pace (see MockState.bearer_pace_bps).
    pub fn set_bearer_pace_bps(&self, bps: Option<f64>) {
        *self.state.bearer_pace_bps.lock().unwrap() = bps;
    }
}

impl Drop for TusMock {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Holds the per-session writer lock; released on every exit path.
struct PatchGuard<'a>(&'a AtomicBool);
impl Drop for PatchGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct Request {
    method: String,
    path: String,
    content_length: u64,
    upload_offset: Option<u64>,
    upload_length: Option<u64>,
    head_len: usize,
}

/// Read one request head (through the blank line). Returns None on EOF.
async fn read_head(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
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
    let head_len = buf.len();
    let head = String::from_utf8_lossy(&buf).into_owned();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
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
    let upload_length = head.lines().find_map(|l| {
        let (name, value) = l.split_once(':')?;
        if !name.eq_ignore_ascii_case("Upload-Length") {
            return None;
        }
        value.trim().parse::<u64>().ok()
    });
    Some(Request {
        method,
        path,
        content_length,
        upload_offset,
        upload_length,
        head_len,
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

async fn persist(session: &Session, data: Vec<u8>, wire: &AtomicU64) {
    wire.fetch_add(data.len() as u64, Ordering::SeqCst);
    let mut store = session.store.lock().unwrap();
    let base = session.offset.load(Ordering::SeqCst) as usize;
    if store.len() < base + data.len() {
        store.resize(base + data.len(), 0);
    }
    store[base..base + data.len()].copy_from_slice(&data);
    drop(store);
    session
        .offset
        .fetch_add(data.len() as u64, Ordering::SeqCst);
}

/// Sleep the current caller until its `bytes` fit into the shared bearer's
/// timeline. Reads are claimed in arrival order (the slot chain) — a fair
/// first-order bearer model.
async fn claim_bearer_slots(state: &MockState, bytes: u64) {
    let total = *state.bearer_pace_bps.lock().unwrap();
    let Some(total) = total else { return };
    if total <= 0.0 {
        return;
    }
    let span = Duration::from_secs_f64(bytes as f64 / total);
    let now = tokio::time::Instant::now();
    let start = {
        let mut slot = state.bearer_slot.lock().unwrap();
        let s = (*slot).max(now);
        *slot = s + span;
        s
    };
    if start > now {
        tokio::time::sleep_until(start).await;
    }
    tokio::time::sleep(span).await;
}

async fn handle(mut stream: TcpStream, state: Arc<MockState>) {
    let Some(req) = read_head(&mut stream).await else {
        return;
    };
    state
        .wire_bytes
        .fetch_add(req.head_len as u64, Ordering::SeqCst);
    match req.method.as_str() {
        "POST" => {
            // Creation: mint a UNIQUE session, 201 + absolute Location.
            let n = state.next_id.fetch_add(1, Ordering::SeqCst);
            let path = format!("/files/ium-mock-{n}");
            let length = req.upload_length.unwrap_or(0);
            state.sessions.lock().unwrap().insert(
                path.clone(),
                Arc::new(Session {
                    store: Mutex::new(Vec::new()),
                    offset: AtomicU64::new(0),
                    length,
                    patch_busy: AtomicBool::new(false),
                }),
            );
            let location = format!("http://{}{path}", stream.local_addr().expect("local"));
            respond(
                &mut stream,
                "201 Created",
                &[("Location", location), ("Tus-Resumable", "1.0.0".into())],
            )
            .await;
        }
        "HEAD" => {
            let Some(session) = state.sessions.lock().unwrap().get(&req.path).cloned() else {
                respond(&mut stream, "404 Not Found", &[]).await;
                return;
            };
            respond(
                &mut stream,
                "200 OK",
                &[
                    (
                        "Upload-Offset",
                        session.offset.load(Ordering::SeqCst).to_string(),
                    ),
                    ("Upload-Length", session.length.to_string()),
                    ("Tus-Resumable", "1.0.0".into()),
                    ("Cache-Control", "no-store".into()),
                ],
            )
            .await;
        }
        "PATCH" => {
            let Some(session) = state.sessions.lock().unwrap().get(&req.path).cloned() else {
                respond(&mut stream, "404 Not Found", &[]).await;
                return;
            };
            // tusd wire fact: a PATCH whose Upload-Offset ≠ the session's
            // offset is rejected 409 AT THE REQUEST HEAD — the body is not
            // consumed.
            if req.upload_offset != Some(session.offset.load(Ordering::SeqCst)) {
                respond(
                    &mut stream,
                    "409 Conflict",
                    &[
                        (
                            "Upload-Offset",
                            session.offset.load(Ordering::SeqCst).to_string(),
                        ),
                        ("Tus-Resumable", "1.0.0".into()),
                    ],
                )
                .await;
                return;
            }
            // One writer per session: a concurrent PATCH gets 423, and the
            // adapter's Busy-wait absorbs it (issue #44).
            if session
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
            let _guard = PatchGuard(&session.patch_busy);
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
                    // stall ENDS when the client aborts (RST/EOF reaches
                    // the socket) or after 120 s — the lock releases with
                    // it either way.
                    let mut sink = [0u8; 512];
                    let _ = tokio::time::timeout(Duration::from_secs(120), stream.read(&mut sink))
                        .await;
                }
                PatchBehavior::KillAfterReading { bytes } => {
                    let mut taken = 0u64;
                    let mut buf = vec![0u8; 16 * 1024];
                    while taken < bytes {
                        let want = (bytes - taken).min(buf.len() as u64) as usize;
                        let read = match tokio::time::timeout(
                            Duration::from_secs(5),
                            stream.read(&mut buf[..want]),
                        )
                        .await
                        {
                            Ok(Ok(n)) if n > 0 => n,
                            _ => break,
                        };
                        persist(&session, buf[..read].to_vec(), &state.wire_bytes).await;
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
                        persist(&session, buf[..read].to_vec(), &state.wire_bytes).await;
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
                                session.offset.load(Ordering::SeqCst).to_string(),
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
                        persist(&session, buf[..read].to_vec(), &state.wire_bytes).await;
                        remaining -= read as u64;
                        // THE UPLINK: per-connection pace, then the shared
                        // bearer's slot (both None on a healthy link).
                        if let Some(bps) = pace {
                            tokio::time::sleep(Duration::from_secs_f64(read as f64 / bps)).await;
                        }
                        claim_bearer_slots(&state, read as u64).await;
                    }
                    state.patches_completed.fetch_add(1, Ordering::SeqCst);
                    respond(
                        &mut stream,
                        "204 No Content",
                        &[
                            (
                                "Upload-Offset",
                                session.offset.load(Ordering::SeqCst).to_string(),
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
