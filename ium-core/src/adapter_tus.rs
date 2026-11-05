//! TUS 1.0.0 adapter (issue #10) — the second real transport, and the first
//! with **byte-level resume**: the server offset is authoritative, so the
//! zero-re-upload guarantee is per-BYTE, not per-part (contrast S3's atomic
//! parts, module doc in `adapter_s3`).
//!
//! Wire facts rehearsed live on tusd v2.10.1 before any Rust (pinned on
//! issue #10), asserted — not assumed — by `tests/adapter_tus.rs`:
//! * every request carries `Tus-Resumable: 1.0.0`; missing → `412`;
//! * creation: `POST <endpoint>` + `Upload-Length` → `201` + `Location`
//!   (absolute on tusd; the spec allows relative — both resolved);
//! * transfer: `PATCH <loc>` + `Upload-Offset` +
//!   `Content-Type: application/offset+octet-stream` → `204` +
//!   `Upload-Offset` (the server-acked new offset — the ONLY honest
//!   `bytes_confirmed`);
//! * `409` = offset conflict = the crash-recovery path: an in-flight PATCH
//!   was killed after the server acked but before the journal write. The
//!   adapter absorbs the drift — HEAD, re-PATCH from the server offset, and
//!   report `final_offset - lease.resume_from` so the journal converges to
//!   server truth (the drifted bytes WERE server-acked, just never
//!   journaled). The dance is BOUNDED (#43): up to `MAX_REALIGN_ROUNDS`
//!   absorption rounds per lease — on slow links a realign can die the same
//!   death as the attempt that spawned it (the server kept OUR bytes), and
//!   that is drift to absorb again, never a loud client bug;
//! * `404` = unknown session → `SessionLost` (→ Reattach);
//! * wire-fact delta (#33/#37): @tus/server answers an early-rejected PATCH
//!   (unknown file, offset conflict) by writing the error status and
//!   closing the connection WITHOUT consuming the request body — a client
//!   still streaming a large body may therefore observe a connection reset
//!   (`error sending request`) instead of the 404/409. tusd drains and
//!   answers normally. The adapter thus never declares from the reset
//!   alone: after a PATCH transport failure it HEADs the session and lets
//!   the SERVER's answer classify — `404/410` → `SessionLost` (→ Reattach),
//!   offset moved past the attempt → the `Conflict` realign dance (the 409
//!   it never saw), otherwise → `Transient` (retries stay counted);
//! * verify is a **read-only** HEAD (`offset == length == source_size`) —
//!   TUS needs no commit gate (contrast ADR-010's S3 finalize-if-needed);
//! * tusd v2.10.1 offers no `checksum` extension — not used (verify is
//!   offset-based; per-request checksums are a future negotiation).
//!
//! **The concatenation lane (issue #50, M8 rung 3).** tusd advertises
//! `concatenation` in `OPTIONS` (rehearsed live, vectors pinned on #50 and
//! asserted by `tests/tus_concat.rs`): partial creation
//! (`Upload-Concat: partial`) ×N, N concurrent PATCHes on independent
//! sessions, then ONE final POST (`Upload-Concat: final;<u1> <u2> …`) the
//! server materializes eagerly. Wire facts that shape the code:
//! * the final REQUIRES every partial complete (`400
//!   ERR_UPLOAD_NOT_FINISHED` otherwise) — finalize runs only at the
//!   all-Confirmed boundary;
//! * the final is IMMUTABLE (`403 ERR_MODIFY_FINAL`) and a re-POST creates
//!   a SECOND final — the journaled final URL is never re-POSTed;
//! * verify gates on the FINAL URL (`offset == length == source_size`).
//!
//! Sessions are keyed by `spec.endpoint`, journaled via the probe event
//! (ADR-002); `restore_session` is the reattach path — identical shape to
//! S3. The token carries the TOPOLOGY: a plain URL is the classic
//! single-session lane; a JSON blob (`{"concat":true,"partials":[…],
//! "final":…}`) is the concatenation lane — `parse_session_token` is the
//! dispatch, so old journals (plain URLs) re-derive byte-identically. The
//! empty `[(0, 0)]` chunk (issue #5) confirms via HEAD instead of a
//! pointless empty PATCH — still a real server round-trip (classic lane).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::adapter_http::Counting;
use crate::chunk::Lease;
use crate::JobSpec;

const TUS_VERSION: &str = "1.0.0";
const TUS_CONTENT_TYPE: &str = "application/offset+octet-stream";
/// Idle-progress bound for tus requests (issue #42). The OLD client had a
/// 600 s TOTAL request timeout — a wall that killed every legal piece below
/// ~8.7 KB/s (a 5 MiB piece needs ≥ 600 s there), making the tus lane
/// structurally non-convergent at the owner's 3 KB/s tunnel moment. The
/// wall is GONE: no total request timeout exists anymore. What remains is
/// PROGRESS-based: `read_timeout` cuts a request only when no bytes have
/// arrived for this long (a stalled response is indistinguishable from a
/// dead tunnel), while a steadily-draining lane rides for as long as it
/// takes. Body-side stalls (buffers full, wire dead mid-send) are owned by
/// the SUPERVISION layer — the frozen-link rule (#28) measures zero bytes
/// and reshapes/aborts; the request layer must stop owning duration.
/// MEASURED (tus_slow_lane drills): a moved()-based watchdog cannot make
/// this distinction — kernel/hyper buffers absorb MiB before backpressure,
/// so a healthy slow wire LOOKS frozen to a client-side counter; the
/// response-side read timeout is the honest tripwire. A rate-scaled lease
/// guard from #41's probe measurements remains the planned backstop.
const TUS_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounded realign rounds per lease (issue #43): the dance absorbs own-drift
/// (a killed PATCH's bytes landing on the server) up to this many times;
/// beyond it the failure is `Transient` — a counted retry that re-queues
/// the chunk from journal truth. Four rounds absorb every realistic
/// slow-link death spiral while keeping a pathological server from spinning
/// the dance forever.
const MAX_REALIGN_ROUNDS: u32 = 4;

/// `Upload-Offset` header → u64. Absent, empty, or malformed = no truth
/// (never a silent 0 — an offset is a server acknowledgment).
pub fn parse_upload_offset(header: Option<&str>) -> Result<u64, AdapterError> {
    let Some(raw) = header else {
        return Err(AdapterError::Unknown(
            "no Upload-Offset header where one was required".into(),
        ));
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AdapterError::Unknown("empty Upload-Offset header".into()));
    }
    raw.parse::<u64>()
        .map_err(|_| AdapterError::Unknown(format!("malformed Upload-Offset header: '{raw}'")))
}

/// Resolve a creation response's `Location` against the creation endpoint.
/// Absolute references pass through; root-relative merge onto the origin;
/// path-relative merge onto the base directory (RFC 3986 §5.2 merge).
pub fn resolve_location(base: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    if let Some(rest) = location.strip_prefix('/') {
        // origin = scheme://authority
        if let Some(after_scheme) = base.split_once("://") {
            if let Some(slash) = after_scheme.1.find('/') {
                let origin = &base[..after_scheme.0.len() + 3 + slash];
                return format!("{origin}/{rest}");
            }
            return format!("{base}/{rest}");
        }
    }
    if base.ends_with('/') {
        format!("{base}{location}")
    } else {
        format!("{base}/{location}")
    }
}

/// The journaled session TOPOLOGY (issue #50): what the probe event's
/// `session_token` actually describes. A plain URL is the classic
/// single-session lane (every pre-#50 journal); the JSON shape is the
/// concatenation lane. Parsing never fails: anything unrecognized stays a
/// classic URL — a token is server truth, never discarded.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionToken {
    /// One sequential-offset session (the pre-#50 shape).
    Classic(String),
    /// N partial sessions + the materialized final URL once finalize ran.
    Concat {
        /// The partial upload URLs, in final-list order (fact 5).
        partials: Vec<String>,
        /// The final upload's URL — `None` until finalize materializes it.
        final_url: Option<String>,
    },
}

/// Encode a concat session as the journaled JSON token.
pub fn encode_concat_token(partials: &[String], final_url: Option<&str>) -> String {
    serde_json::json!({
        "concat": true,
        "partials": partials,
        "final": final_url,
    })
    .to_string()
}

/// Parse a journaled session token — plain URL = classic, JSON = concat.
/// Malformed JSON falls back to classic: the token is server truth.
pub fn parse_session_token(raw: &str) -> SessionToken {
    if !raw.starts_with('{') {
        return SessionToken::Classic(raw.to_string());
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) if v.get("concat").and_then(|c| c.as_bool()) == Some(true) => {
            let partials = v
                .get("partials")
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|u| u.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if partials.is_empty() {
                return SessionToken::Classic(raw.to_string());
            }
            SessionToken::Concat {
                partials,
                final_url: v.get("final").and_then(|f| f.as_str()).map(String::from),
            }
        }
        _ => SessionToken::Classic(raw.to_string()),
    }
}

/// The byte-range partition behind the lane (issue #50): partial *i* owns
/// `[i·L, min((i+1)·L, total))`, `L = ceil(total/n)` — a pure tiling with
/// no gaps and no overlaps. Chunks (planned or resplit) route their
/// intersection with each range to that partial's session; a chunk
/// spanning a boundary is two segments on two sessions, confirmed once.
pub fn concat_partition(total: u64, n: u32) -> Vec<(u64, u64)> {
    let n = n.max(1) as u64;
    let piece = total.div_ceil(n);
    (0..n)
        .map(|i| {
            let start = (i * piece).min(total);
            let end = ((i + 1) * piece).min(total);
            (start, end)
        })
        .collect()
}

/// TUS 1.0.0 client. No credentials by design (tusd-style servers); auth
/// headers are a future negotiation when a real deployment needs them.
pub struct TusAdapter {
    client: reqwest::Client,
    sessions: Mutex<HashMap<String, SessionToken>>,
    /// Per-partial serialization (issue #50): a partial session is ONE
    /// sequential-offset resource — two leased chunks routed to the same
    /// partial (after a resplit) must not race its offsets. Different
    /// partials run concurrently: the SESSION stays sequential, the LANE
    /// is parallel. Lazily sized per endpoint.
    partial_locks: Mutex<HashMap<String, Vec<std::sync::Arc<AsyncMutex<()>>>>>,
    /// Bonded paths (issue #51): partial *i* rides bond `i % P` — its
    /// creation POST and every PATCH/HEAD go out that bond's client
    /// (source-bound; multi-NIC routing) toward that bond's endpoint
    /// override (the drill rig's per-bearer proxy; production single-
    /// endpoint hosts leave it `None`). EMPTY = single-path mode:
    /// everything rides `client` exactly as before (#50 unchanged).
    bonds: Vec<Bond>,
}

/// One bonded path's HTTP surface (issue #51).
struct Bond {
    client: reqwest::Client,
    /// Endpoint override for this path's sessions (`None` = the job's
    /// endpoint — the production shape).
    endpoint: Option<String>,
}

impl TusAdapter {
    /// A fresh adapter with its own HTTP client and an EMPTY session map —
    /// sessions are restored from the journal via `reattach` (#12), never
    /// guessed.
    ///
    /// Client timeouts (issue #42): NO total request timeout — the 600 s
    /// wall made the lane structurally non-convergent below ~8.7 KB/s.
    /// Instead: a connect timeout for dead endpoints and a READ timeout
    /// (per-read idle) as the progress-based tripwire; body-side stalls
    /// belong to the supervision layer (see `TUS_IDLE_TIMEOUT`).
    pub fn new() -> Self {
        Self::with_idle(TUS_IDLE_TIMEOUT)
    }

    /// A fresh adapter with a custom idle-progress window. Production runs
    /// `new` (30 s); drills compress the window to make the semantics
    /// observable in seconds (tests/tus_slow_lane.rs).
    pub fn with_idle(idle: Duration) -> Self {
        Self {
            client: Self::build_client(idle, None),
            sessions: Mutex::new(HashMap::new()),
            partial_locks: Mutex::new(HashMap::new()),
            bonds: Vec::new(),
        }
    }

    fn build_client(idle: Duration, path: Option<&crate::netpath::NetPath>) -> reqwest::Client {
        let builder = match path {
            None => Ok(reqwest::Client::builder()),
            Some(p) => crate::netpath::client_builder(p),
        };
        builder
            .unwrap_or_else(|e| {
                panic!(
                    "bonded client for '{}' cannot be declared on this OS: {e}",
                    path.map(|p| p.name()).unwrap_or_default()
                )
            })
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(idle)
            .build()
            .expect("reqwest client builds")
    }

    /// A fresh adapter whose CONCAT lane rides bonded paths (issue #51):
    /// partial *i* is created via bond `i % P` — its `Location` (the
    /// server's Host echo) pins every later PATCH/HEAD to the same
    /// bearer, and the source binding routes the packets the same way in
    /// production (one endpoint, many NICs). The classic lane and the
    /// final materialization stay on the default client (one session =
    /// one path; the final POST is a single request on any bearer).
    ///
    /// `(NetPath, Option<endpoint>)`: the override exists for drills
    /// where each bearer is its own proxy in front of one tusd.
    pub fn with_bonds(
        bonds: Vec<(crate::netpath::NetPath, Option<String>)>,
    ) -> Result<Self, crate::netpath::PathError> {
        let mut built = Vec::with_capacity(bonds.len());
        for (path, endpoint) in bonds {
            let client = crate::netpath::client_builder(&path)?
                .connect_timeout(Duration::from_secs(30))
                .read_timeout(TUS_IDLE_TIMEOUT)
                .build()?;
            built.push(Bond { client, endpoint });
        }
        Ok(Self {
            client: Self::build_client(TUS_IDLE_TIMEOUT, None),
            sessions: Mutex::new(HashMap::new()),
            partial_locks: Mutex::new(HashMap::new()),
            bonds: built,
        })
    }

    /// Bond `i`'s HTTP surface: (client, endpoint override). Single-path
    /// mode (no bonds) = (the default client, None) — byte-for-byte the
    /// pre-#51 routing.
    fn bond(&self, i: usize) -> (&reqwest::Client, Option<&str>) {
        if self.bonds.is_empty() {
            (&self.client, None)
        } else {
            let b = &self.bonds[i % self.bonds.len()];
            (&b.client, b.endpoint.as_deref())
        }
    }

    /// The upload URL for a job — the journaled probe event's session token,
    /// ENCODED (classic = the plain URL, concat = the JSON shape; the
    /// journal and this map carry the same string).
    pub fn session_for(&self, endpoint: &str) -> Option<String> {
        let t = self.sessions.lock().unwrap().get(endpoint).cloned()?;
        Some(match t {
            SessionToken::Classic(url) => url,
            SessionToken::Concat {
                partials,
                final_url,
            } => encode_concat_token(&partials, final_url.as_deref()),
        })
    }

    /// Reattach a fresh adapter instance to an existing upload (the host
    /// replays the journaled probe event — ADR-002). Both topologies parse.
    pub fn restore_session(&self, endpoint: &str, location: String) {
        self.sessions
            .lock()
            .unwrap()
            .insert(endpoint.to_string(), parse_session_token(&location));
    }

    /// The per-partial lock vector for an endpoint, grown lazily to `n`.
    fn locks_for(&self, endpoint: &str, n: usize) -> Vec<std::sync::Arc<AsyncMutex<()>>> {
        let mut map = self.partial_locks.lock().unwrap();
        let v = map.entry(endpoint.to_string()).or_default();
        while v.len() < n {
            v.push(std::sync::Arc::new(AsyncMutex::new(())));
        }
        v.clone()
    }

    /// OPTIONS discovery (issue #50): does the server ADVERTISE the
    /// concatenation extension? One request per probe; journaled in the
    /// probe event, re-derived from the journal on resume — never re-probed
    /// mid-flight (ADR-002). FAILURE IS NOT FATAL: any OPTIONS error or
    /// unexpected status means "no advertisement" — the classic lane
    /// proceeds byte-for-byte as pre-#50 (a server that never implemented
    /// OPTIONS, or a momentary hiccup, must not kill a job that creation
    /// can still serve; the immediately-following creation POST surfaces
    /// any REAL server failure on its own).
    async fn discover_concat(&self, endpoint: &str) -> Result<bool, AdapterError> {
        let resp = match self
            .client
            .request(reqwest::Method::OPTIONS, endpoint)
            .header("Tus-Resumable", TUS_VERSION)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(false),
        };
        Ok(matches!(resp.status().as_u16(), 200 | 204)
            && resp
                .headers()
                .get("tus-extension")
                .and_then(|v| v.to_str().ok())
                .map(|ext| {
                    ext.split(',')
                        .any(|e| e.trim().eq_ignore_ascii_case("concatenation"))
                })
                .unwrap_or(false))
    }

    /// The CLASSIC creation — the pre-#50 probe body, byte-for-byte: one
    /// POST, one session, `max_parallel: Some(1)`.
    async fn create_classic(
        &self,
        spec: &JobSpec,
        concatenation: bool,
    ) -> Result<ProbeReport, AdapterError> {
        let resp = self
            .client
            .post(&spec.endpoint)
            .header("Tus-Resumable", TUS_VERSION)
            .header("Upload-Length", spec.source_size.to_string())
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("creation failed: {e}")))?;
        match resp.status().as_u16() {
            201 => {
                let raw = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        AdapterError::Unknown("201 creation without a Location header".into())
                    })?
                    .to_string();
                let location = resolve_location(&spec.endpoint, &raw);
                self.restore_session(&spec.endpoint, location.clone());
                Ok(ProbeReport {
                    uniform_block: None,
                    resumable: true,
                    max_chunk_bytes: None,
                    // Issue #13: TUS is ONE sequential-offset resource.
                    // Concurrent PATCHes to different offsets 409-realign
                    // into Unresumable — the protocol itself caps parallel
                    // uploads at 1 per session, so the adapter DECLARES it
                    // (ADR-011 channel) and the planner obeys.
                    max_parallel: Some(1),
                    // Issue #44: tus has NO protocol minimum — the adapter
                    // DECLARES a 64 KiB lane floor (ADR-011 channel). At the
                    // owner's 3 KB/s bad-moment rate a piece confirms in
                    // ~30 s; the per-PATCH header overhead at this size is
                    // measured in the #45 ladder drill, not asserted.
                    min_chunk_bytes: Some(64 * 1024),
                    session_token: Some(location),
                    concatenation,
                })
            }
            404 | 410 => Err(AdapterError::SessionLost(format!(
                "creation endpoint {} is gone ({})",
                spec.endpoint,
                resp.status()
            ))),
            413 => Err(AdapterError::Unknown(format!(
                "creation refused: Upload-Length {} exceeds the server's Tus-Max-Size",
                spec.source_size
            ))),
            412 => Err(AdapterError::Unknown(
                "creation 412: server rejected Tus-Resumable preconditions".into(),
            )),
            s if s >= 500 => Err(AdapterError::Transient(format!("creation -> {s}"))),
            s => Err(AdapterError::Unknown(format!("creation -> {s}"))),
        }
    }

    /// Create ONE partial session (issue #50 fact 2): `Upload-Concat:
    /// partial` + the range's `Upload-Length` → `201` + `Location`.
    /// Create ONE partial session (issue #50 fact 2): `Upload-Concat:
    /// partial` + the range's `Upload-Length` → `201` + `Location`.
    /// Bonded (issue #51): bond `bond_idx`'s client + endpoint override —
    /// the creation decides the partial's bearer for life (the server's
    /// Location echoes the Host this POST arrived on).
    async fn create_partial(
        &self,
        spec: &JobSpec,
        bond_idx: usize,
        length: u64,
    ) -> Result<String, AdapterError> {
        let (client, endpoint_override) = self.bond(bond_idx);
        let endpoint = endpoint_override.unwrap_or(&spec.endpoint);
        let resp = client
            .post(endpoint)
            .header("Tus-Resumable", TUS_VERSION)
            .header("Upload-Concat", "partial")
            .header("Upload-Length", length.to_string())
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("partial creation failed: {e}")))?;
        match resp.status().as_u16() {
            201 => {
                let raw = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        AdapterError::Unknown("201 partial creation without a Location".into())
                    })?
                    .to_string();
                Ok(resolve_location(endpoint, &raw))
            }
            404 | 410 => Err(AdapterError::SessionLost(format!(
                "creation endpoint {} is gone ({})",
                endpoint,
                resp.status()
            ))),
            412 => Err(AdapterError::Unknown(
                "partial creation 412: server rejected Tus-Resumable preconditions".into(),
            )),
            400 => Err(AdapterError::Unknown(format!(
                "partial creation 400: {}",
                String::from_utf8_lossy(&resp.bytes().await.unwrap_or_default())
            ))),
            s if s >= 500 => Err(AdapterError::Transient(format!("partial creation -> {s}"))),
            s => Err(AdapterError::Unknown(format!("partial creation -> {s}"))),
        }
    }

    fn io_err(e: std::io::Error) -> AdapterError {
        AdapterError::Transient(format!("source read failed: {e}"))
    }

    /// `HEAD <loc>` → `(Upload-Length, Upload-Offset)`. The offset is the
    /// server's durable truth; the length is what creation declared.
    async fn head(&self, location: &str) -> Result<(u64, u64), AdapterError> {
        self.head_via(&self.client, location).await
    }

    /// `head` over a specific bond's client (issue #51): the partial's
    /// own path rides its own bearer for the alignment HEAD too.
    async fn head_via(
        &self,
        client: &reqwest::Client,
        location: &str,
    ) -> Result<(u64, u64), AdapterError> {
        let resp = client
            .head(location)
            .header("Tus-Resumable", TUS_VERSION)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("HEAD failed: {e}")))?;
        match resp.status().as_u16() {
            200 => {
                let offset = parse_upload_offset(
                    resp.headers()
                        .get("upload-offset")
                        .and_then(|v| v.to_str().ok()),
                )?;
                let length = resp
                    .headers()
                    .get("upload-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                Ok((length.unwrap_or(0), offset))
            }
            404 | 410 => Err(AdapterError::SessionLost(format!(
                "upload {location} is gone ({} )",
                resp.status()
            ))),
            s if s >= 500 => Err(AdapterError::Transient(format!("HEAD -> {s}"))),
            s => Err(AdapterError::Unknown(format!("HEAD -> {s}"))),
        }
    }

    /// One PATCH. `upload_offset` is the PARTIAL-LOCAL offset the server
    /// expects; `source_offset` is where the bytes live in the SOURCE file.
    /// They coincide on the classic lane (one session == the whole file);
    /// on the concat lane partial *i*'s local offset L maps to absolute
    /// source offset `base + L` (issue #50). `409` is not an
    /// `AdapterError` — it is the protocol's crash-recovery signal and the
    /// caller (the realign dance) must see it as data.
    async fn patch(
        &self,
        spec: &JobSpec,
        location: &str,
        upload_offset: u64,
        source_offset: u64,
        end: u64,
        progress: &ProgressReporter,
    ) -> Result<u64, PatchFailure> {
        self.patch_via(
            &self.client,
            spec,
            location,
            upload_offset,
            source_offset,
            end,
            progress,
        )
        .await
    }

    /// `patch` over a specific bond's client (issue #51).
    #[allow(clippy::too_many_arguments)]
    async fn patch_via(
        &self,
        client: &reqwest::Client,
        spec: &JobSpec,
        location: &str,
        upload_offset: u64,
        source_offset: u64,
        end: u64,
        progress: &ProgressReporter,
    ) -> Result<u64, PatchFailure> {
        let span = end - upload_offset;
        let mut file = tokio::fs::File::open(&spec.source_path)
            .await
            .map_err(Self::io_err)
            .map_err(PatchFailure::Adapter)?;
        file.seek(std::io::SeekFrom::Start(source_offset))
            .await
            .map_err(Self::io_err)
            .map_err(PatchFailure::Adapter)?;
        let reader = file.take(span);
        let body = reqwest::Body::wrap_stream(FramedRead::new(
            Counting {
                inner: reader,
                progress: progress.clone(),
            },
            BytesCodec::new(),
        ));

        let resp = match client
            .patch(location)
            .header("Tus-Resumable", TUS_VERSION)
            .header("Upload-Offset", upload_offset.to_string())
            .header("Content-Type", TUS_CONTENT_TYPE)
            .header("Content-Length", span.to_string())
            .body(body)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                // Transport failure is ambiguous by itself: a real network
                // blip (→ Transient, retry), an idle-progress cut (→ the
                // server may hold SOME of the body it read before the
                // socket died), and @tus/server's early-reject socket close
                // (#33/#37 — the 404/409 never reaches the client while the
                // body is still streaming) look identical to hyper. Ask the
                // server before declaring — its answer, not the socket's
                // mood, classifies (verify-before-declare, for error paths
                // too):
                //
                // * HEAD 404/410 → SessionLost (→ Reattach): the session
                //   is REALLY gone;
                // * HEAD 200 with an offset ≠ the attempted one → the
                //   PATCH was rejected or partially consumed without
                //   acking to us — the 409 we never saw. `Conflict` sends
                //   the caller into the realign dance from the server's
                //   offset (this call's flush is reckoned as waste there);
                // * anything else (server at exactly our attempted offset
                //   — the PATCH took nothing — or HEAD failing too) →
                //   Transient: a genuine wire problem, retries counted.
                let msg = e.to_string();
                return Err(match self.head_via(client, location).await {
                    Err(AdapterError::SessionLost(_)) => {
                        PatchFailure::Adapter(AdapterError::SessionLost(format!(
                            "PATCH transport-failed and HEAD confirms the session is gone: {msg}"
                        )))
                    }
                    Ok((_, server_offset)) if server_offset != upload_offset => {
                        PatchFailure::Conflict
                    }
                    _ => PatchFailure::Adapter(AdapterError::Transient(format!(
                        "PATCH failed: {msg}"
                    ))),
                });
            }
        };
        match resp.status().as_u16() {
            204 => parse_upload_offset(
                resp.headers()
                    .get("upload-offset")
                    .and_then(|v| v.to_str().ok()),
            )
            .map_err(PatchFailure::Adapter),
            409 => Err(PatchFailure::Conflict),
            404 | 410 => Err(PatchFailure::Adapter(AdapterError::SessionLost(format!(
                "upload {location} is gone ({})",
                resp.status()
            )))),
            412 => Err(PatchFailure::Adapter(AdapterError::Unknown(
                "412: server rejected our Tus-Resumable preconditions — client bug".into(),
            ))),
            423 => Err(PatchFailure::Adapter(AdapterError::Busy(
                "423 Locked: the upload resource is serialized — another writer or an \
                 abandoned request's server-side drain holds it (issue #44)"
                    .into(),
            ))),
            s if s >= 500 => Err(PatchFailure::Adapter(AdapterError::Transient(format!(
                "PATCH -> {s}"
            )))),
            s => Err(PatchFailure::Adapter(AdapterError::Unknown(format!(
                "PATCH -> {s}"
            )))),
        }
    }

    /// Push one SEGMENT `[seg.0, seg.1)` (absolute source bytes) through
    /// the partial session that owns `[base, …)` (issue #50). The #10/#43
    /// rules apply PER SESSION: entry-HEAD alignment, the bounded realign
    /// dance, the waste reckoning — all scoped to the partial's own offset
    /// space, with the segment's confirm reported in ABSOLUTE bytes. The
    /// per-partial lock is held across the whole dance: a partial is ONE
    /// sequential-offset resource (the lane's parallelism lives BETWEEN
    /// partials).
    ///
    /// The SERVER'S OFFSET IS THE TRUTH, whatever it is: on a fresh
    /// partial (or after a resplit re-routed chunks) it can legally sit
    /// BELOW the segment's local start — the PATCH then pushes from the
    /// server's offset up to the segment end, streaming the source from
    /// the mapped absolute position (`base + local`). Those extra bytes
    /// belong to the neighboring chunk's span (the journal confirms them
    /// there, via its own confirm-before-send HEAD), the wire pushes them
    /// exactly once, and the confirm counts ONLY this segment's span —
    /// the accounting stays per-call honest (issue #37's arithmetic,
    /// scoped per segment).
    #[allow(clippy::too_many_arguments)] // (spec, partial, lock, span, base) — the concat shape
    async fn push_segment(
        &self,
        spec: &JobSpec,
        partial_idx: usize,
        partial_url: &str,
        lock: &AsyncMutex<()>,
        seg: (u64, u64),
        base: u64,
        progress: &ProgressReporter,
    ) -> Result<u64, AdapterError> {
        // The partial's OWN bond (issue #51): its HEADs and PATCHes ride
        // the bearer its creation chose (Location echo) via the same
        // source-bound client.
        let (client, _) = self.bond(partial_idx);
        let _guard = lock.lock().await;
        let local_end = seg.1 - base;

        // THE ENTRY HEAD — per session (the #43 discipline, concat edition):
        // the journal can be behind THIS partial's server truth.
        let (_, entry) = self.head_via(client, partial_url).await?;
        // Confirm-before-send: this partial already holds the whole segment.
        if entry >= local_end {
            return Ok(seg.1 - seg.0);
        }

        let first_attempt_from = progress.moved();
        let mut from = entry;
        let mut realigns: u32 = 0;
        let final_local = loop {
            match self
                .patch_via(
                    client,
                    spec,
                    partial_url,
                    from,
                    base + from,
                    local_end,
                    progress,
                )
                .await
            {
                Ok(offset) => break offset,
                Err(PatchFailure::Adapter(e)) => return Err(e),
                Err(PatchFailure::Conflict) => {
                    realigns += 1;
                    if realigns > MAX_REALIGN_ROUNDS {
                        return Err(AdapterError::Transient(format!(
                            "tus realign dance exceeded {MAX_REALIGN_ROUNDS} rounds on this segment — re-queues from journal truth"
                        )));
                    }
                    let (_, server_offset) = self.head(partial_url).await?;
                    if server_offset < from.min(entry) {
                        return Err(AdapterError::Unresumable(format!(
                            "partial offset {server_offset} behind the entry offset {entry} — the server lost confirmed bytes",
                        )));
                    }
                    if server_offset >= local_end {
                        // The segment is fully server-held (own-drift or a
                        // same-partial neighbor chunk's confirmed bytes —
                        // the lock makes them ordered, the journal makes
                        // them owned): confirm in full.
                        break local_end;
                    }
                    from = server_offset;
                }
            }
        };
        // THE WASTE RECKONING — the same settled arithmetic as the classic
        // path (issue #37), scoped to this segment: bytes pushed beyond what
        // the server persisted between the entry offset and the final one.
        let pushed = progress.moved() - first_attempt_from;
        progress.waste(pushed.saturating_sub(final_local - entry));
        // The confirm counts ONLY this segment's span: the server may hold
        // more than the segment end (a same-partial neighbor's bytes) —
        // those belong to the neighbor's journal confirm, never to ours.
        let confirmed_abs = (base + final_local).min(seg.1);
        Ok(confirmed_abs.saturating_sub(seg.0))
    }
}

/// Why one PATCH did not produce a server-acked offset.
enum PatchFailure {
    /// 409: the server's offset is elsewhere — the crash-recovery signal.
    Conflict,
    /// Everything else, already mapped into the engine's error taxonomy.
    Adapter(AdapterError),
}

impl Default for TusAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl TransferAdapter for TusAdapter {
    fn name(&self) -> &'static str {
        "tus"
    }

    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }

    /// The classic probe PLUS discovery (issue #50): one OPTIONS for the
    /// capability (journaled in the report), then the pre-#50 creation
    /// body — one session, `max_parallel: Some(1)`. Plain `probe` NEVER
    /// activates the concat lane: only the host's `probe_fleet` intent
    /// does (the wire sequence gains exactly one OPTIONS request; every
    /// semantic is unchanged, and the pre-#50 wire suite passes as-is).
    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        let concatenation = self.discover_concat(&spec.endpoint).await?;
        self.create_classic(spec, concatenation).await
    }

    /// The fleet-seeded probe (issue #50, ADR-011 channel): the host offers
    /// its journal-derived seed; the adapter obeys the SERVER first — no
    /// advertisement, or a seed too small to parallelize (or a 0-byte
    /// source, where there is nothing to split) → the classic lane,
    /// byte-for-byte. With the advertisement: N = seed partial sessions
    /// (fact 2) and the declaration `max_parallel: Some(N)` rides the
    /// #13 channel — the prober's measured fleet (#48) and AIMD (#49)
    /// adapt WITHIN it.
    async fn probe_fleet(
        &self,
        spec: &JobSpec,
        fleet_seed: u32,
    ) -> Result<ProbeReport, AdapterError> {
        let concatenation = self.discover_concat(&spec.endpoint).await?;
        if !concatenation || fleet_seed < 2 || spec.source_size == 0 {
            return self.create_classic(spec, concatenation).await;
        }
        // The adapter's own honest ceiling: no more partial sessions than
        // the declared lane floor can justify — a 64 KiB partial is the
        // smallest useful session (the #44 floor), below it parallelism is
        // pure header overhead. A huge host seed on a tiny file collapses.
        let max_useful = spec.source_size.div_ceil(64 * 1024).max(1) as u32;
        let n = fleet_seed.clamp(1, max_useful);
        let partition = concat_partition(spec.source_size, n);
        let mut partials = Vec::with_capacity(partition.len());
        for (i, (start, end)) in partition.iter().enumerate() {
            // Bonded lane (issue #51): partial i rides bond i % P — the
            // creation POST decides the bearer for the session's life.
            partials.push(self.create_partial(spec, i, end - start).await?);
        }
        let token = encode_concat_token(&partials, None);
        self.sessions
            .lock()
            .unwrap()
            .insert(spec.endpoint.clone(), parse_session_token(&token));
        self.locks_for(&spec.endpoint, partials.len());
        Ok(ProbeReport {
            uniform_block: None,
            resumable: true,
            max_chunk_bytes: None,
            max_parallel: Some(partials.len() as u32),
            min_chunk_bytes: Some(64 * 1024),
            session_token: Some(token),
            concatenation: true,
        })
    }

    /// Reattach (issue #12): replay the journaled probe event's session
    /// token into the session map — both topologies (classic URL, concat
    /// JSON). Pure in-memory restore — no network, no new POST creation
    /// (that would start a second upload resource and orphan the first).
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.restore_session(endpoint, token.to_string());
        Ok(())
    }

    /// Materialize the final upload (issue #50): `POST` + `Upload-Concat:
    /// final;<u1> <u2> …` (fact 5). Idempotent per session: a journaled
    /// final URL is REUSED, never re-POSTed (fact 5b — a re-POST would
    /// create a SECOND final). Returns the URL the verify gate checks, or
    /// `None` on the classic lane (nothing to materialize).
    async fn finalize(&self, spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        let token = {
            let map = self.sessions.lock().unwrap();
            map.get(&spec.endpoint).cloned()
        };
        let (partials, known) = match token {
            Some(SessionToken::Concat {
                partials,
                final_url,
            }) => (partials, final_url),
            _ => return Ok(None),
        };
        if let Some(url) = known {
            return Ok(Some(url));
        }
        let header = format!("final;{}", partials.join(" "));
        let resp = self
            .client
            .post(&spec.endpoint)
            .header("Tus-Resumable", TUS_VERSION)
            .header("Upload-Concat", header)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("final creation failed: {e}")))?;
        match resp.status().as_u16() {
            201 => {
                let raw = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        AdapterError::Unknown("201 final creation without a Location".into())
                    })?
                    .to_string();
                let url = resolve_location(&spec.endpoint, &raw);
                self.sessions.lock().unwrap().insert(
                    spec.endpoint.clone(),
                    SessionToken::Concat {
                        partials,
                        final_url: Some(url.clone()),
                    },
                );
                Ok(Some(url))
            }
            // Fact 8c: a partial is not finished — the caller violated the
            // all-Confirmed boundary. Loud, never retried blindly.
            400 => Err(AdapterError::Unknown(format!(
                "final creation 400: {}",
                String::from_utf8_lossy(&resp.bytes().await.unwrap_or_default()).trim()
            ))),
            404 | 410 => Err(AdapterError::SessionLost(format!(
                "final creation 404: a partial in the list is gone ({})",
                resp.status()
            ))),
            412 => Err(AdapterError::Unknown(
                "final creation 412: server rejected Tus-Resumable preconditions".into(),
            )),
            s if s >= 500 => Err(AdapterError::Transient(format!("final creation -> {s}"))),
            s => Err(AdapterError::Unknown(format!("final creation -> {s}"))),
        }
    }

    /// The link probe, lane-shaped (issue #57): the ADAPTER owns its wire,
    /// so the host's probing boundary asks the adapter it already holds.
    /// The tus lane delegates to the free-standing `prober::probe_link`
    /// (unchanged measurement: the payload's own prefix, one canary session
    /// then K concurrent ones) — the manager's `spec.adapter == "tus"`
    /// string match is dead.
    async fn probe_link(
        &self,
        spec: &JobSpec,
        k: u32,
        sample_hint_bps: Option<f64>,
    ) -> Result<Option<crate::prober::LinkProfile>, AdapterError> {
        let path = std::path::Path::new(&spec.source_path);
        crate::prober::probe_link(path, spec.source_size, &spec.endpoint, k, sample_hint_bps)
            .await
            .map(Some)
            .map_err(|e| AdapterError::Transient(format!("tus link probe: {e}")))
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let token = self
            .sessions
            .lock()
            .unwrap()
            .get(&spec.endpoint)
            .cloned()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no tus session for endpoint; restore from the journaled probe event".into(),
                )
            })?;

        // ---- the CONCATENATION lane (issue #50) --------------------------
        if let SessionToken::Concat { partials, .. } = &token {
            if lease.end <= lease.resume_from {
                // The empty `[(0,0)]` chunk on the concat lane: the lease
                // spans nothing, so no partial is touched; the job's final
                // POST + final-URL verify are the honest round-trips.
                return Ok(ChunkConfirm {
                    idx: lease.idx,
                    bytes_confirmed: 0,
                    adapter_token: None,
                });
            }
            let partition = concat_partition(spec.source_size, partials.len() as u32);
            let locks = self.locks_for(&spec.endpoint, partials.len());
            let mut confirmed = 0u64;
            let mut advanced: Vec<usize> = Vec::new();
            for (i, &(base, pend)) in partition.iter().enumerate() {
                let rs = lease.resume_from.max(base);
                let re = lease.end.min(pend);
                if rs >= re {
                    continue; // this partial's range is outside the lease
                }
                confirmed += self
                    .push_segment(spec, i, &partials[i], &locks[i], (rs, re), base, progress)
                    .await?;
                advanced.push(i);
            }
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: confirmed,
                adapter_token: Some(format!("partials {advanced:?} advanced")),
            });
        }

        // ---- the CLASSIC lane (pre-#50, unchanged) ------------------------
        let SessionToken::Classic(location) = token else {
            return Err(AdapterError::SessionLost(
                "unreachable: non-classic token fell through the concat arm".into(),
            ));
        };

        // THE ENTRY HEAD — the alignment point (CI wire-suite finding, #43
        // follow-up): the journal can be BEHIND the server at call entry
        // (a previous call died after the server acked bytes that never
        // reached the journal — crash recovery's whole reason to exist).
        // PATCHing from the journal offset would 409 blindly; HEADing first
        // makes the server's truth the starting point, saves the doomed
        // round-trip, and makes the waste reckoning EXACT (server-persisted
        // during this call = final − entry, with no ambiguity between
        // pre-existing drift and this call's consumed bytes).
        let (_, entry_offset) = self.head(&location).await?;
        if entry_offset < lease.resume_from {
            return Err(AdapterError::Unresumable(format!(
                "server offset {entry_offset} behind journaled {} — the server lost confirmed bytes",
                lease.resume_from
            )));
        }
        // Confirm-before-send shapes (issue #5's empty span, and #44's
        // resplit-induced drift): the server already holds the whole span —
        // a real server round-trip verified it. Confirm in full; zero
        // pointless PATCHes.
        if lease.end <= entry_offset {
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.resume_from,
                adapter_token: Some(entry_offset.to_string()),
            });
        }

        // Issue #30 wire honesty: snapshot before the first PATCH so the
        // rejected attempts' flush can be reclassified exactly.
        let first_attempt_from = progress.moved();
        // THE REALIGN DANCE (issues #10, #30, #43): a conflict means the
        // server's offset moved mid-call — on a single-worker lease that is
        // OUR OWN drift (a killed PATCH's body landed on the server after
        // the socket died; tusd persists what arrived). The server's offset
        // is the truth: absorb, re-PATCH from it, and converge. BOUNDED at
        // MAX_REALIGN_ROUNDS rounds per lease — beyond that the failure is
        // Transient (a counted retry that re-queues the chunk from journal
        // truth). On this single-writer lane there is no foreign writer, so
        // a server offset at or beyond the lease end is ALSO our own drift
        // (resplit-induced, issue #44): the piece confirms in full and the
        // excess flows to the following pieces.
        let mut from = entry_offset;
        let mut realigns: u32 = 0;
        let final_offset = loop {
            match self
                .patch(spec, &location, from, from, lease.end, progress)
                .await
            {
                Ok(offset) => break offset,
                Err(PatchFailure::Adapter(e)) => return Err(e),
                Err(PatchFailure::Conflict) => {
                    realigns += 1;
                    if realigns > MAX_REALIGN_ROUNDS {
                        return Err(AdapterError::Transient(format!(
                            "tus realign dance exceeded {MAX_REALIGN_ROUNDS} rounds on this lease — re-queues from journal truth"
                        )));
                    }
                    let (_, server_offset) = self.head(&location).await?;
                    if server_offset < entry_offset {
                        return Err(AdapterError::Unresumable(format!(
                            "server offset {server_offset} behind the entry offset {entry_offset} — the server lost confirmed bytes",
                        )));
                    }
                    if server_offset >= lease.end {
                        // RESPLIT-INDUCED DRIFT (issue #44): see the
                        // entry-HEAD shape above — the piece confirms in
                        // full; the excess belongs to the FOLLOWING pieces.
                        break lease.end;
                    }
                    from = server_offset;
                }
            }
        };

        if final_offset < lease.resume_from {
            return Err(AdapterError::Unresumable(format!(
                "server acked offset {final_offset} behind the lease start {}",
                lease.resume_from
            )));
        }
        // THE WASTE RECKONING — settled arithmetic, not an instant snapshot
        // (issue #37): the killed attempts' last chunks can still land in
        // `moved()` after `patch()` returns (the body is counted on hyper's
        // connection task — an 8 KiB chunk once flipped CI's exact assert).
        // Reckon ONCE, after the dance settles: during this call the
        // server's durable truth moved from the ENTRY offset to
        // `final_offset`; every byte this call pushed BEYOND that is, by
        // construction, flushed-nowhere (a rejected attempt's socket flush
        // or a dead attempt's never-consumed tail) — exactly waste,
        // whenever its count landed. Own-drift bytes (server-persisted
        // slices of killed attempts) sit INSIDE `final_offset - entry`:
        // they are CONFIRMED, never double-counted as waste — and
        // pre-existing drift (journal behind the server at entry) sits
        // BELOW the entry offset: it is neither pushed by this call nor
        // subtracted from it (the CI drift-realign wire test pins this).
        let pushed = progress.moved() - first_attempt_from;
        progress.waste(pushed.saturating_sub(final_offset - entry_offset));
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: final_offset - lease.resume_from,
            adapter_token: Some(final_offset.to_string()),
        })
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        // Read-only integrity gate. Classic lane: the server holds exactly
        // source_size bytes on the ONE session. Concat lane (issue #50):
        // the gate is the FINAL URL (fact 6 — eager materialization; fact
        // 8b — the final is the immutable resource); before finalize runs
        // there IS no final, and verify refuses to lie about it.
        let token = self
            .sessions
            .lock()
            .unwrap()
            .get(&spec.endpoint)
            .cloned()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no tus session for endpoint; restore from the journaled probe event".into(),
                )
            })?;
        match token {
            SessionToken::Concat {
                final_url: None, ..
            } => Ok(VerifyReport {
                ok: false,
                detail: Some(
                    "concat lane not finalized: no final URL yet — finalize must run at the \
                     all-Confirmed boundary before verify can gate the final resource"
                        .into(),
                ),
            }),
            SessionToken::Concat {
                final_url: Some(url),
                ..
            } => {
                let (length, offset) = self.head(&url).await?;
                if offset == spec.source_size && length == spec.source_size {
                    Ok(VerifyReport {
                        ok: true,
                        detail: Some(format!(
                            "tus concat final {url}: offset {offset} == length == source_size"
                        )),
                    })
                } else {
                    Ok(VerifyReport {
                        ok: false,
                        detail: Some(format!(
                            "concat final holds {offset} of {} bytes (Upload-Length {length})",
                            spec.source_size
                        )),
                    })
                }
            }
            SessionToken::Classic(location) => {
                let (length, offset) = self.head(&location).await?;
                if offset == spec.source_size && length == spec.source_size {
                    Ok(VerifyReport {
                        ok: true,
                        detail: Some(format!("tus offset {offset} == length == source_size")),
                    })
                } else {
                    Ok(VerifyReport {
                        ok: false,
                        detail: Some(format!(
                            "server holds {offset} of {} bytes (Upload-Length {length})",
                            spec.source_size
                        )),
                    })
                }
            }
        }
    }
}
