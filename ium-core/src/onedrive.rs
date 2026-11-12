//! The OneDrive lane (#110) — IUM's own Entra identity, PKCE public client,
//! and the provider's own integrity hash on the verify gate.
//!
//! Unlike the bridge family (#63/#82), OneDrive needs NO client secret: this
//! is a PKCE public client (RFC 7636) registered in IUM's own Entra app
//! ("IUM Desktop", all Microsoft account users). The client id is a PUBLIC
//! identifier — the lane's security is the loopback redirect + PKCE, never
//! client secrecy. The refresh token rides the grant (`offline_access` —
//! WITHOUT it Entra issues no refresh token at all) and lives in the #81
//! SecretStore.
//!
//! Wire contract (Microsoft Graph, official docs; LIVE rehearsal lands on
//! the #110 ledger before this lane closes):
//! * `createUploadSession → PUT fragments (Content-Range) → the FINAL
//!   fragment's PUT answers 200/201 with the driveItem` — there is NO
//!   separate finish call on Graph; the last byte IS the commit;
//! * every fragment except the last MUST be a multiple of **320 KiB**; the
//!   planned chunk is 5 MiB (16 × 320 KiB, inside the official 5–10 MiB
//!   band) — the prescribed-geometry channel (#56);
//! * an out-of-order fragment answers **416**; the session's own
//!   `nextExpectedRanges` (a GET on the self-authenticated `uploadUrl`) is
//!   the server's truth — the #10 realign pattern, server wins;
//! * quickXorHash = the official algorithm (160-bit circular vector, shift
//!   11, the 8-byte little-endian length XORed into bytes 12–20, LE
//!   serialization, base64) — matched against an independent Python
//!   implementation AND the independent PyPI `quickxorhash` (12/12 vectors)
//!   before anything was pinned.
//!
//! HONEST LIMITS: one sequential-offset session per job (`max_parallel: 1`);
//! the `uploadUrl` is a self-authenticated URL and is journaled as the
//! session token (it IS the resumable handle — `nextExpectedRanges` replays
//! it); sessions vanish when abandoned — a reattach onto a dead session is
//! an honest `ExpiredSession` and the re-probe restarts the job from zero
//! on the same journal. Entra access tokens are 60–90 min with a RANDOMIZED
//! TTL — the bearer refreshes lazily (60 s margin) and on one forced 401
//! retry; Entra rotates refresh tokens on use: the rotated grant wins
//! in-process (`TokenSet::merged_with` keeps the stored one when the answer
//! omits it) and the vault's stored grant wins again on restart — if Entra
//! invalidated it meanwhile, the honest `AuthRequired` surfaces and the
//! one-click re-auth fixes it (completed fragments survive via the
//! journaled `uploadUrl`, exactly the YouTube lane's documented scope).
//! Empty files are refused at probe with an honest sentence (the session
//! protocol has no zero-byte fragment shape; the simple-upload path is a
//! separate lane, not a silent improvisation here).

use std::time::Duration;

use reqwest::header::{HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::google::TokenSet;
use crate::JobSpec;

/// The registry key.
pub const LANE: &str = "onedrive";
/// IUM's own Entra application (client) id — a PUBLIC identifier (PKCE
/// public client; no secret exists on this lane by design, and the
/// registration's client-credential surface stays empty for the lane's
/// lifetime).
pub const CLIENT_ID: &str = "c3a906be-d85b-4409-b3cd-bc152ae58550";
/// The loopback port the redirect comes back to — the lane convention
/// (google 8737 → dropbox 8738 → onedrive 8739). The Entra registration is
/// the bare `http://localhost` native-client form, so Entra matches ANY
/// port at runtime (RFC 8252 loopback rule); the engine still pins ONE
/// fixed port for deterministic UX.
pub const REDIRECT_PORT: u16 = 8739;
/// The redirect URI the engine sends (port-flexible on the Entra side by
/// registration, fixed here by convention).
pub const REDIRECT_URI: &str = "http://localhost:8739";
/// The prescribed uniform block (#56 channel): the planned chunk size, 16 ×
/// 320 KiB — inside the official 5–10 MiB band for upload sessions.
pub const UNIFORM_BLOCK: u64 = 5 * 1024 * 1024;
/// The Graph HARD rule: every fragment except the last is a multiple of
/// 320 KiB. The contract test pins `UNIFORM_BLOCK % FRAGMENT_ALIGN == 0`.
pub const FRAGMENT_ALIGN: u64 = 320 * 1024;
/// The wire's per-fragment cap (official recommended maximum).
pub const MAX_CHUNK: u64 = 60 * 1024 * 1024;
/// The scope set — `offline_access` is REQUIRED (no refresh token without
/// it), `Files.ReadWrite` is the upload surface, `User.Read` rides the
/// default consent shape.
pub const SCOPE: &str = "offline_access Files.ReadWrite User.Read";

const AUTHORIZE: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// The quickXorHash algorithm constants (the official C# reference).
const WIDTH: usize = 160;
const SHIFT: usize = 11;
const BITS_IN_LAST_CELL: usize = 32;

/// The redirect URI the lane speaks (one spelling, byte-identical on the
/// authorize request and the token exchange).
pub fn redirect_uri() -> &'static str {
    REDIRECT_URI
}

/// Standard base64 WITH padding — the Graph `quickXorHash` string form
/// (20 bytes → 28 chars). The PKCE twin in `google` is the URL-safe
/// unpadded spelling; these are different alphabets on purpose.
fn b64std(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The official quickXorHash streaming hasher — a faithful port of the C#
/// reference on learn.microsoft.com ("Code Snippets: QuickXorHash
/// Algorithm"): a 160-bit circular vector held as 2 full u64 cells + a
/// 32-bit last cell, each byte XORed at bit position `shift + 11·i` with
/// the byte's spill past a cell boundary XORed into the next cell (and
/// past bit 160 back into cell 0 — the wrap is an XOR, never an OR: an
/// OR-wrap fold is order-dependent and silently diverges on multi-cycle
/// inputs). The per-call shift advance happens ONCE per `update` (by
/// `shift·(cbSize % 160)`), not per byte — the streaming contract the
/// adapter's fragment-wise hashing rides.
pub struct QuickXorHasher {
    data: [u64; 3],
    shift_so_far: usize,
    length: u64,
}

impl Default for QuickXorHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickXorHasher {
    /// A fresh hasher.
    pub fn new() -> Self {
        Self {
            data: [0; 3],
            shift_so_far: 0,
            length: 0,
        }
    }

    /// Feed one span of bytes (an arbitrary-size `HashCore` call).
    pub fn update(&mut self, bytes: &[u8]) {
        let cb = bytes.len();
        if cb == 0 {
            return; // an empty HashCore call is a no-op on the reference too
        }
        let mut vector_array_index = self.shift_so_far / 64;
        let mut vector_offset = self.shift_so_far % 64;
        let iterations = cb.min(WIDTH);
        for i in 0..iterations {
            let is_last_cell = vector_array_index == 2;
            let bits_in_vector_cell: usize = if is_last_cell { BITS_IN_LAST_CELL } else { 64 };
            if vector_offset <= bits_in_vector_cell - 8 {
                // the byte fits inside this cell — XOR every WIDTH-th byte
                // of the span at the same bit position
                let mut j = i;
                while j < cb {
                    self.data[vector_array_index] ^= (bytes[j] as u64) << vector_offset;
                    j += WIDTH;
                }
            } else {
                // the byte splits across the cell boundary: the low part
                // XORs here, the high part XORs into the next cell (circular
                // at the 160-bit edge — index2 wraps to 0)
                let index1 = vector_array_index;
                let index2 = if is_last_cell {
                    0
                } else {
                    vector_array_index + 1
                };
                let low = (bits_in_vector_cell - vector_offset) as u32;
                let mut xored_byte = 0u8;
                let mut j = i;
                while j < cb {
                    xored_byte ^= bytes[j];
                    j += WIDTH;
                }
                self.data[index1] ^= (xored_byte as u64) << vector_offset;
                self.data[index2] ^= (xored_byte as u64) >> low;
            }
            vector_offset += SHIFT;
            while vector_offset >= bits_in_vector_cell {
                vector_array_index = if is_last_cell {
                    0
                } else {
                    vector_array_index + 1
                };
                vector_offset -= bits_in_vector_cell;
            }
        }
        self.shift_so_far = (self.shift_so_far + SHIFT * (cb % WIDTH)) % WIDTH;
        self.length = self.length.wrapping_add(cb as u64);
    }

    /// The digest: 20 bytes, LE serialization of the three cells, then the
    /// 8-byte little-endian length XORed into bytes 12–20.
    pub fn finalize_bytes(&self) -> [u8; 20] {
        let mut rgb = [0u8; 20];
        rgb[0..8].copy_from_slice(&self.data[0].to_le_bytes());
        rgb[8..16].copy_from_slice(&self.data[1].to_le_bytes());
        rgb[16..20].copy_from_slice(&self.data[2].to_le_bytes()[..4]);
        let lb = self.length.to_le_bytes();
        for i in 0..8 {
            rgb[12 + i] ^= lb[i];
        }
        rgb
    }

    /// The digest in the Graph's string form (standard base64, 28 chars).
    pub fn finalize(&self) -> String {
        b64std(&self.finalize_bytes())
    }
}

/// One-shot quickXorHash — the Graph string form. Vectors in the tests were
/// cross-checked against an independent Python implementation built from
/// the official algorithm page AND the independent PyPI `quickxorhash`
/// (12/12 match) before anything was pinned.
pub fn quick_xor_hash(data: &[u8]) -> String {
    let mut h = QuickXorHasher::new();
    h.update(data);
    h.finalize()
}

/// The streaming twin of [`quick_xor_hash`] over a file — verify reads the
/// source in fixed windows and never buffers the whole file. The algorithm
/// is call-boundary independent (the contract test drives 1-byte and
/// 7-byte splits over megabytes to prove it).
async fn file_quick_xor_hash(path: &str) -> std::io::Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = QuickXorHasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize())
}

/// Percent-encode everything RFC 3986's unreserved set does not cover —
/// query values (scope, state, redirect) must survive every OS's
/// browser-argument plumbing.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The colon-path spelling: unreserved + `/` stay literal (the path's
/// separators are structure, not data), everything else encodes —
/// `root:/видео/my clip.mp4:` must survive as one URL.
fn path_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The consent-screen URL: Entra's real page opens in the user's browser,
/// the redirect returns to the loopback listener on the lane's fixed port,
/// and the grant carries a refresh token (`offline_access` — the Entra
/// scope that makes refresh tokens exist at all). PKCE S256.
pub fn authorize_url(verifier: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type=code&state={}&code_challenge={}&code_challenge_method=S256&scope={}",
        percent_encode(CLIENT_ID),
        percent_encode(REDIRECT_URI),
        percent_encode(state),
        crate::google::pkce_challenge(verifier),
        percent_encode(SCOPE),
    )
}

/// The token client — public client, no secret anywhere.
#[derive(Debug, Clone, Default)]
pub struct OneDriveClient;

impl OneDriveClient {
    fn http() -> Client {
        Client::builder()
            .user_agent(concat!("IUM/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("the reqwest client builds with rustls")
    }

    /// The token exchange: `authorization_code` + the PKCE verifier. The
    /// redirect must byte-match the authorize request's.
    pub async fn exchange_code(&self, code: &str, verifier: &str) -> Result<TokenSet, String> {
        let params = [
            ("code", code),
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier),
        ];
        Self::token_answer(
            Self::http()
                .post(TOKEN)
                .form(&params)
                .send()
                .await
                .map_err(|e| format!("the token exchange could not reach Entra: {e}"))?,
        )
        .await
    }

    /// The refresh grant — the long-lived grant in, a fresh access token
    /// out. Entra access tokens live 60–90 min with a randomized TTL; the
    /// answer may ROTATE the refresh token (single-use rotation on public
    /// clients) — the caller merges via [`TokenSet::merged_with`] so an
    /// answer without a new refresh token keeps the stored one.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet, String> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
        ];
        Self::token_answer(
            Self::http()
                .post(TOKEN)
                .form(&params)
                .send()
                .await
                .map_err(|e| format!("the token refresh could not reach Entra: {e}"))?,
        )
        .await
    }

    async fn token_answer(resp: reqwest::Response) -> Result<TokenSet, String> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("the token answer could not be read: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "Entra refused the token request ({status}): {}",
                body.chars().take(200).collect::<String>()
            ));
        }
        TokenSet::parse(&body)
    }
}

/// The lane's configuration — ONE credential: the stored grant the browser
/// sign-in wrote. No client fields exist (the client id is compiled; there
/// is no secret on a PKCE public client).
#[derive(Debug, Clone)]
pub struct OneDriveConfig {
    /// The stored grant: access token + refresh token + expiry.
    pub token: TokenSet,
}

impl OneDriveConfig {
    /// The required-credential check, naming the env var it expected.
    pub fn required(token: Option<&str>) -> Result<String, AdapterError> {
        token
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AdapterError::AuthRequired("IUM_ONEDRIVE_TOKEN not set".into()))
    }

    /// The pure constructor: a token JSON that is not a token is an honest
    /// refusal (a pasted cookie must never look like a signed-in lane).
    pub fn from_parts(token_json: &str) -> Result<Self, AdapterError> {
        let token = TokenSet::parse(&Self::required(Some(token_json))?).map_err(|e| {
            AdapterError::AuthRequired(format!(
                "the OneDrive sign-in token is unusable — connect once again. ({e})"
            ))
        })?;
        Ok(Self { token })
    }

    /// The env constructor (`IUM_ONEDRIVE_TOKEN`; the vault's apply_env
    /// fills it from the sign-in's stored grant).
    pub fn from_env() -> Result<Self, AdapterError> {
        Self::from_parts(&std::env::var("IUM_ONEDRIVE_TOKEN").unwrap_or_default())
    }
}

/// The session truth, mutable across confirms and reattaches.
#[derive(Default)]
struct SessionState {
    /// The Graph upload session's self-authenticated `uploadUrl` (the
    /// journaled probe token's living form — it IS the resumable handle).
    upload_url: Option<String>,
    /// The committed receipt (destination path + server quickXorHash) the
    /// completing fragment's driveItem carried.
    committed: Option<(String, String)>,
    /// The access token the metadata calls ride NOW (refreshed lazily).
    access_token: String,
    /// Its absolute expiry (unix seconds); the refresh fires 60 s early.
    access_expires_at: Option<u64>,
}

/// The OneDrive adapter — one connected account, one lane.
pub struct OneDriveAdapter {
    cfg: OneDriveConfig,
    http: Client,
    state: std::sync::RwLock<SessionState>,
}

fn req_err(e: reqwest::Error) -> AdapterError {
    AdapterError::Transient(format!("transport: {e}"))
}

fn io_err(e: std::io::Error) -> AdapterError {
    AdapterError::Transient(format!("local io: {e}"))
}

/// The destination path inside the user's drive. The create form's endpoint
/// (an absolute path starting with `/`) wins; otherwise the source's file
/// name lands at the drive root — the same filename discipline the Dropbox
/// lane's app-folder rule rides (here the scope is the full drive, so the
/// destination is displayed at connect time by the shell).
fn destination_path(spec: &JobSpec) -> String {
    let endpoint = spec.endpoint.trim();
    if endpoint.starts_with('/') {
        return endpoint.to_string();
    }
    let name = spec
        .source_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("upload.bin");
    let name = name.trim();
    if name.is_empty() {
        "/upload.bin".into()
    } else {
        format!("/{name}")
    }
}

/// A wire error's classification — the Graph shapes this lane meets.
fn classify(status: StatusCode, body: &str) -> AdapterError {
    if status == StatusCode::UNAUTHORIZED {
        return AdapterError::AuthRequired(format!(
            "OneDrive refused the credential ({status}) — connect once again: {}",
            body.chars().take(160).collect::<String>()
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return AdapterError::Throttled {
            retry_after_ms: None,
            message: format!("rate: {}", body.chars().take(160).collect::<String>()),
        };
    }
    if status.is_server_error() {
        return AdapterError::Transient(format!(
            "OneDrive answered {status}: {}",
            body.chars().take(160).collect::<String>()
        ));
    }
    // A session that answered 404/410 is gone (abandoned past its lifetime)
    // — the honest restart path, never a silent re-create behind the
    // journal's back.
    if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
        return AdapterError::ExpiredSession(format!(
            "the upload session is gone from OneDrive ({status}): {}",
            body.chars().take(160).collect::<String>()
        ));
    }
    AdapterError::Unknown(format!(
        "OneDrive answered {status}: {}",
        body.chars().take(200).collect::<String>()
    ))
}

/// The server's next expected byte out of an upload session's
/// `nextExpectedRanges` — each entry is `"start-"` (everything from start
/// is missing) or `"start-end"` (that span is missing). The session is a
/// strictly sequential resource in this lane, so the FIRST missing byte is
/// the minimum start over the entries.
fn next_expected_byte(ranges: &[serde_json::Value]) -> Option<u64> {
    let mut min: Option<u64> = None;
    for r in ranges {
        let s = r.as_str()?;
        let head = s.split('-').next()?;
        if let Ok(v) = head.trim().parse::<u64>() {
            min = Some(match min {
                Some(m) if m < v => m,
                _ => v,
            });
        }
    }
    min
}

/// Pull the driveItem's quickXorHash out of a completion or metadata
/// answer (`file.hashes.quickXorHash`).
fn quick_xor_from_item(v: &serde_json::Value) -> Option<String> {
    v.get("file")?
        .get("hashes")?
        .get("quickXorHash")?
        .as_str()
        .map(str::to_string)
}

/// The wire-byte counter over a seeked source — the shared adapter pattern
/// (S3, HTTP, YouTube, Dropbox ride the same shape).
struct Counting<R> {
    inner: R,
    progress: ProgressReporter,
}

impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Counting<R> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match std::pin::Pin::new(&mut this.inner).poll_read(cx, buf) {
            std::task::Poll::Ready(Ok(())) => {
                let n = (buf.filled().len() - before) as u64;
                if n > 0 {
                    this.progress.add(n);
                }
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl OneDriveAdapter {
    /// Production constructor: the config from env, fresh token carried.
    pub fn new(cfg: OneDriveConfig) -> Self {
        let http = Client::builder()
            .user_agent(concat!("IUM/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60 * 60))
            .tcp_nodelay(true)
            .build()
            .expect("the reqwest client builds with rustls");
        Self {
            cfg,
            http,
            state: std::sync::RwLock::new(SessionState::default()),
        }
    }

    /// A fresh bearer header, refreshing lazily on expiry (the stored
    /// grant's refresh token is the durable credential; the vault keeps the
    /// original — a restart re-reads it. Entra may rotate the refresh token
    /// on use; the in-process answer wins until the next restart, the same
    /// honest scope the YouTube and Dropbox lanes document).
    async fn bearer(&self) -> Result<String, AdapterError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        {
            let st = self.state.read().expect("state lock");
            if let Some(exp) = st.access_expires_at {
                if now + 60 < exp {
                    return Ok(st.access_token.clone());
                }
            }
        }
        self.force_refresh().await
    }

    /// The unconditional refresh (the 401 path): one attempt, honest
    /// refusal on exhaustion. The answer's refresh token (Entra's rotation)
    /// merges over the stored one — an answer without a new refresh token
    /// keeps the stored grant alive.
    async fn force_refresh(&self) -> Result<String, AdapterError> {
        let Some(rt) = self.cfg.token.refresh_token.clone() else {
            return Err(AdapterError::AuthRequired(
                "the OneDrive sign-in expired and carries no refresh token — connect once again"
                    .into(),
            ));
        };
        let fresh = OneDriveClient
            .refresh(&rt)
            .await
            .map_err(AdapterError::AuthRequired)?;
        let merged = self.cfg.token.merged_with(fresh);
        let access = merged.access_token.clone();
        let exp = merged.expires_at;
        {
            let mut st = self.state.write().expect("state lock");
            st.access_token = access.clone();
            st.access_expires_at = exp;
        }
        Ok(access)
    }

    /// The journaled source-size check the other adapters share.
    async fn source_len(&self, spec: &JobSpec) -> Result<u64, AdapterError> {
        let meta = tokio::fs::metadata(&spec.source_path)
            .await
            .map_err(io_err)?;
        if !meta.is_file() {
            return Err(AdapterError::Corruption(format!(
                "the source {} is not a file anymore",
                spec.source_path
            )));
        }
        if meta.len() != spec.source_size {
            return Err(AdapterError::Corruption(format!(
                "the source changed underneath the job: journal says {} bytes, disk says {}",
                spec.source_size,
                meta.len()
            )));
        }
        Ok(meta.len())
    }

    /// One Graph GET with the 401-refresh-once dance.
    async fn graph_get(
        &self,
        url: &str,
    ) -> Result<Result<serde_json::Value, (StatusCode, String)>, AdapterError> {
        for attempt in 0..2 {
            let token = if attempt == 0 {
                self.bearer().await?
            } else {
                self.force_refresh().await?
            };
            let resp = self
                .http
                .get(url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(req_err)?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                continue; // one forced-refresh retry, then the honest refusal
            }
            if status.is_success() {
                let parsed = serde_json::from_str(&body).map_err(|e| {
                    AdapterError::Unknown(format!("the Graph answer is not JSON: {e}"))
                })?;
                return Ok(Ok(parsed));
            }
            return Ok(Err((status, body)));
        }
        unreachable!("the loop returns on every branch")
    }

    /// The read-only credential check (#81): `GET /me/drive` — the
    /// metadata read the Test button drives, never an upload session it
    /// could orphan.
    pub async fn check_credentials(&self) -> Result<(), AdapterError> {
        match self.graph_get(&format!("{GRAPH}/me/drive")).await? {
            Ok(_) => Ok(()),
            Err((status, body)) => Err(classify(status, &body)),
        }
    }

    /// Open the upload session — the probe event's journaled token. The
    /// `uploadUrl` is a SELF-AUTHENTICATED URL (its own auth rides the
    /// query): the fragment PUTs never need the bearer, which is exactly
    /// why a 60–90-min access-token expiry cannot interrupt a long job
    /// mid-session.
    async fn create_session(&self, spec: &JobSpec) -> Result<String, AdapterError> {
        let path = destination_path(spec);
        let encoded = path_encode(path.trim_start_matches('/'));
        let url = format!("{GRAPH}/me/drive/root:/{encoded}:/createUploadSession");
        let payload = serde_json::json!({
            "item": {"@microsoft.graph.conflictBehavior": "replace"}
        });
        for attempt in 0..2 {
            let token = if attempt == 0 {
                self.bearer().await?
            } else {
                self.force_refresh().await?
            };
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .header(CONTENT_TYPE, "application/json")
                .body(payload.to_string())
                .send()
                .await
                .map_err(req_err)?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                continue; // one forced-refresh retry, then the honest refusal
            }
            if !status.is_success() {
                return Err(classify(status, &body));
            }
            let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                AdapterError::Unknown(format!("the createUploadSession answer is not JSON: {e}"))
            })?;
            let upload_url = v
                .get("uploadUrl")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .ok_or_else(|| {
                    AdapterError::Unknown(
                        "the createUploadSession answer carries no uploadUrl".into(),
                    )
                })?;
            return Ok(upload_url);
        }
        unreachable!("the loop returns on every branch")
    }

    /// The session's own truth: GET the self-authenticated `uploadUrl` and
    /// read `nextExpectedRanges` — the #10 realign source (server wins).
    async fn session_next_byte(&self, upload_url: &str) -> Result<Option<u64>, AdapterError> {
        let resp = self.http.get(upload_url).send().await.map_err(req_err)?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            // a dead session answers 404 — the honest restart path
            return Err(classify(status, &body));
        }
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            AdapterError::Unknown(format!("the upload-session answer is not JSON: {e}"))
        })?;
        let ranges = v
            .get("nextExpectedRanges")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(next_expected_byte(&ranges))
    }

    /// One fragment WIRE ATTEMPT (PUT on the self-authenticated
    /// `uploadUrl`, `Content-Range` spelling, streaming body). ONE attempt,
    /// no token: a streaming body cannot be cloned, so the realign budget
    /// lives in the CALLER, which rebuilds the body per attempt (the
    /// same-bytes-per-offset invariant makes every rebuild byte-identical).
    async fn put_fragment_once(
        &self,
        upload_url: &str,
        spec: &JobSpec,
        from: u64,
        end: u64,
        total: u64,
        progress: &ProgressReporter,
    ) -> Result<(StatusCode, String), AdapterError> {
        let span = end - from;
        let file = tokio::fs::File::open(&spec.source_path)
            .await
            .map_err(io_err)?;
        let mut src = file;
        src.seek(std::io::SeekFrom::Start(from))
            .await
            .map_err(io_err)?;
        let stream = tokio_util::codec::FramedRead::new(
            Counting {
                inner: src.take(span),
                progress: progress.clone(),
            },
            tokio_util::codec::BytesCodec::new(),
        );
        let range = format!("bytes {from}-{}/{total}", end - 1);
        let resp = self
            .http
            .put(upload_url)
            .header(
                CONTENT_RANGE,
                HeaderValue::from_str(&range).map_err(|e| {
                    AdapterError::Unknown(format!("the content-range refused: {e}"))
                })?,
            )
            .header(
                CONTENT_LENGTH,
                HeaderValue::from_str(&span.to_string())
                    .expect("a decimal length is always a header value"),
            )
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
            .map_err(req_err)?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Ok((status, text))
    }

    /// Upload one span at an absolute offset, realigning on the wire's 416
    /// (bounded — a realign that lands outside the lease is honest
    /// corruption, never a silent improvisation). The COMPLETING fragment
    /// (span ends at the total) journals the driveItem receipt its 200/201
    /// carried.
    async fn upload_span(
        &self,
        spec: &JobSpec,
        from: u64,
        end: u64,
        total: u64,
        progress: &ProgressReporter,
    ) -> Result<(), AdapterError> {
        let upload_url = self
            .state
            .read()
            .expect("state lock")
            .upload_url
            .clone()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no live session in this process — reattach from the journaled token".into(),
                )
            })?;
        let mut from = from;
        let mut realigns = 0u32;
        loop {
            let (status, body) = self
                .put_fragment_once(&upload_url, spec, from, end, total, progress)
                .await?;
            if status.is_success() {
                if end == total {
                    // the completing fragment: Graph answers 200/201 with
                    // the driveItem — journal the receipt (verify re-reads
                    // the server truth independently)
                    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                        AdapterError::Unknown(format!("the completion answer is not JSON: {e}"))
                    })?;
                    let hash = quick_xor_from_item(&v).unwrap_or_default();
                    let path = v
                        .get("parentReference")
                        .and_then(|p| p.get("path"))
                        .and_then(|p| p.as_str())
                        .map(|p| {
                            // "drive/root:/backup" + name → "/backup/name"
                            let base = p.rsplit(':').next().unwrap_or("");
                            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            format!("{}/{}", base.trim_end_matches('/'), name)
                        })
                        .unwrap_or_else(|| destination_path(spec));
                    let mut st = self.state.write().expect("state lock");
                    st.committed = Some((path, hash));
                }
                return Ok(());
            }
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                // the server is at a different byte: ask the SESSION (the
                // truth), never improvise from the error body
                match self.session_next_byte(&upload_url).await? {
                    Some(correct) if correct >= end => {
                        return Ok(()); // the server already holds our span
                    }
                    Some(correct) if correct <= from && correct < end => {
                        // the server is BEHIND our lease start: bytes before
                        // this lease are missing — the journal and the wire
                        // diverged. Honest corruption, never a re-send of
                        // another chunk's range behind the manager's back.
                        return Err(AdapterError::Corruption(format!(
                            "the session holds {correct} bytes but this chunk starts at {from} — the journal is ahead of the wire"
                        )));
                    }
                    Some(correct) => {
                        realigns += 1;
                        if realigns > 2 {
                            return Err(AdapterError::Corruption(
                                "the session kept correcting its offset beyond the realign budget"
                                    .into(),
                            ));
                        }
                        from = correct; // server wins realign; re-send the tail
                        continue;
                    }
                    None => return Err(classify(status, &body)),
                }
            }
            return Err(classify(status, &body));
        }
    }

    /// The metadata read for verify/finalize — the server truth about one
    /// drive item (size + the hashes family).
    async fn item_metadata(
        &self,
        spec: &JobSpec,
    ) -> Result<Result<serde_json::Value, (StatusCode, String)>, AdapterError> {
        let path = destination_path(spec);
        let encoded = path_encode(path.trim_start_matches('/'));
        let url = format!("{GRAPH}/me/drive/root:/{encoded}:?$select=id,size,file");
        self.graph_get(&url).await
    }
}

#[async_trait::async_trait]
impl TransferAdapter for OneDriveAdapter {
    fn name(&self) -> &'static str {
        LANE
    }

    fn capabilities(&self) -> Capability {
        // Session-based resume; NO dynamic chunks (the prescribed 320 KiB
        // alignment owns the plan) and NO tail-race (the final fragment's
        // PUT is the atomic commit — no data/commit race window).
        Capability::new(Capability::RESUMABLE)
    }

    fn uniform_block(&self) -> Option<u64> {
        Some(UNIFORM_BLOCK)
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        let total = self.source_len(spec).await?;
        if total == 0 {
            return Err(AdapterError::Unknown(
                "empty files are not supported on the OneDrive session lane — the simple-upload path is a separate lane".into(),
            ));
        }
        let upload_url = self.create_session(spec).await?;
        {
            let mut st = self.state.write().expect("state lock");
            st.upload_url = Some(upload_url.clone());
            st.committed = None;
        }
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(MAX_CHUNK),
            min_chunk_bytes: None,
            max_parallel: Some(1),
            session_token: Some(upload_url),
            uniform_block: Some(UNIFORM_BLOCK),
            concatenation: false,
        })
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let total = self.source_len(spec).await?;
        if lease.resume_from >= lease.end {
            // The server already holds this whole segment (a prior process
            // confirmed it and died before journalling). The completing
            // fragment's receipt is finalize's metadata fallback.
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: 0,
                adapter_token: None,
            });
        }
        self.upload_span(spec, lease.resume_from, lease.end, total, progress)
            .await?;
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: lease.end - lease.resume_from,
            adapter_token: None,
        })
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let meta = match self.item_metadata(spec).await? {
            Ok(v) => v,
            Err((status, body)) => {
                return Ok(VerifyReport {
                    ok: false,
                    detail: Some(format!(
                        "the verify read of {} failed ({status}): {}",
                        destination_path(spec),
                        body.chars().take(160).collect::<String>()
                    )),
                });
            }
        };
        let server_hash = quick_xor_from_item(&meta);
        let server_size = meta.get("size").and_then(|s| s.as_u64());
        let Some(server_hash) = server_hash else {
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "the metadata for {} carries no quickXorHash — verify refuses to declare",
                    destination_path(spec)
                )),
            });
        };
        let local = file_quick_xor_hash(&spec.source_path)
            .await
            .map_err(io_err)?;
        let size_ok = server_size.map(|s| s == spec.source_size).unwrap_or(false);
        let ok = server_hash == local && size_ok;
        Ok(VerifyReport {
            ok,
            detail: Some(if ok {
                format!(
                    "OneDrive quickXorHash {server_hash} matches the local computation over {} bytes",
                    spec.source_size
                )
            } else {
                format!(
                    "quickXorHash mismatch — server {server_hash}, local {local}, server size {server_size:?}, journal size {}",
                    spec.source_size
                )
            }),
        })
    }

    async fn reattach(&self, _endpoint: &str, token: &str) -> Result<(), AdapterError> {
        // The journaled token IS the self-authenticated uploadUrl. Never
        // CREATE a session — only adopt the journaled one; the offsets live
        // in the journal and the session's own nextExpectedRanges corrects
        // the first out-of-step fragment.
        {
            let mut st = self.state.write().expect("state lock");
            st.upload_url = Some(token.to_string());
            st.committed = None;
        }
        Ok(())
    }

    async fn finalize(&self, spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        let total = self.source_len(spec).await?;
        if let Some((path, _)) = self.state.read().expect("state lock").committed.clone() {
            return Ok(Some(path));
        }
        // No in-process receipt (a prior process sent the completing
        // fragment and died before journalling): the metadata read is the
        // honest receipt — the item exists at the journal's size.
        match self.item_metadata(spec).await? {
            Ok(meta) => {
                let size_ok = meta.get("size").and_then(|s| s.as_u64()) == Some(total);
                if !size_ok {
                    return Err(AdapterError::Corruption(format!(
                        "the journal completed {total} bytes but OneDrive holds {}",
                        meta.get("size").and_then(|s| s.as_u64()).unwrap_or(0)
                    )));
                }
                let path = destination_path(spec);
                let hash = quick_xor_from_item(&meta).unwrap_or_default();
                let mut st = self.state.write().expect("state lock");
                st.committed = Some((path.clone(), hash));
                Ok(Some(path))
            }
            Err((status, body)) => Err(classify(status, &body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lane_pins_its_wire_constants() {
        assert_eq!(LANE, "onedrive");
        assert_eq!(CLIENT_ID, "c3a906be-d85b-4409-b3cd-bc152ae58550");
        assert_eq!(REDIRECT_PORT, 8739);
        assert_eq!(redirect_uri(), "http://localhost:8739");
        assert_eq!(UNIFORM_BLOCK, 5 * 1024 * 1024);
        assert_eq!(FRAGMENT_ALIGN, 320 * 1024);
        assert_eq!(UNIFORM_BLOCK % FRAGMENT_ALIGN, 0);
        assert_eq!(MAX_CHUNK, 60 * 1024 * 1024);
        assert_eq!(SCOPE, "offline_access Files.ReadWrite User.Read");
    }

    #[test]
    fn the_standard_base64_encoder_matches_the_reference() {
        assert_eq!(b64std(b""), "");
        assert_eq!(b64std(b"f"), "Zg==");
        assert_eq!(b64std(b"fo"), "Zm8=");
        assert_eq!(b64std(b"foo"), "Zm9v");
        assert_eq!(b64std(b"foob"), "Zm9vYg==");
        assert_eq!(b64std(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64std(b"foobar"), "Zm9vYmFy");
        // the empty quickXorHash rides this encoder: 20 zero bytes
        assert_eq!(b64std(&[0u8; 20]), "AAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    }

    #[test]
    fn quick_xor_hash_matches_the_official_vectors() {
        // Vectors cross-checked against an independent Python implementation
        // (scripts/quickxor_ref.py) AND the independent PyPI quickxorhash
        // (12/12 match) — see tests/onedrive_contract.rs for the full set.
        assert_eq!(quick_xor_hash(&[]), "AAAAAAAAAAAAAAAAAAAAAAAAAAA=");
        let pattern = |n: usize| (0..n).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
        assert_eq!(quick_xor_hash(b"a"), "YQAAAAAAAAAAAAAAAQAAAAAAAAA=");
        assert_eq!(quick_xor_hash(b"abc"), "YRDDGAAAAAAAAAAAAwAAAAAAAAA=");
        assert_eq!(
            quick_xor_hash(&pattern(600)),
            "z0rVUrguclftJj1osaWH0yosV44="
        );
        assert_eq!(
            quick_xor_hash(&pattern(1024 * 1024)),
            "pVcWCkWK/HTa4DMyE2B1A1Y9XyM="
        );
        assert_eq!(
            quick_xor_hash(&pattern(5 * 1024 * 1024)),
            "/p/DCDFbDO0I+W4Z5OEzEr6u56Q="
        );
    }

    #[test]
    fn the_streaming_hasher_survives_arbitrary_call_boundaries() {
        // the C# contract: _shiftSoFar advances ONCE per HashCore call —
        // the fragment-wise adapter hashing rides exactly this
        let data: Vec<u8> = (0..(2 * 1024 * 1024 + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        let one_shot = quick_xor_hash(&data);
        for chunk_len in [700 * 1024, 327_680, 160, 7, 1] {
            let mut h = QuickXorHasher::new();
            for chunk in data.chunks(chunk_len) {
                h.update(chunk);
            }
            assert_eq!(
                h.finalize(),
                one_shot,
                "streaming diverged at chunk_len={chunk_len}"
            );
        }
    }

    #[tokio::test]
    async fn the_file_hash_streaming_twin_matches_the_in_memory_one() {
        let dir = std::env::temp_dir().join(format!("ium-onedrive-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("span.bin");
        let data: Vec<u8> = (0..(1024 * 1024 + 1234)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let streamed = file_quick_xor_hash(path.to_str().unwrap()).await.unwrap();
        assert_eq!(streamed, quick_xor_hash(&data));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_authorize_url_carries_pkce_offline_and_the_exact_redirect() {
        let url = authorize_url("verifier-abc", "state-xyz");
        assert!(url.starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"));
        assert!(url.contains("client_id=c3a906be-d85b-4409-b3cd-bc152ae58550"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8739"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=state-xyz"));
        assert!(url.contains("scope=offline_access%20Files.ReadWrite%20User.Read"));
    }

    #[test]
    fn the_config_refuses_a_missing_or_garbage_token_by_name() {
        let e = OneDriveConfig::required(None).unwrap_err();
        assert!(e.to_string().contains("IUM_ONEDRIVE_TOKEN"));
        let e = OneDriveConfig::from_parts("not-json").unwrap_err();
        assert!(matches!(e, AdapterError::AuthRequired(_)));
        OneDriveConfig::from_parts(r#"{"access_token":"a","refresh_token":"r","expires_in":4500}"#)
            .expect("a real token shape parses");
    }

    #[test]
    fn destinations_come_from_the_endpoint_or_the_filename() {
        let spec = |endpoint: &str, path: &str| JobSpec {
            source_path: path.into(),
            source_size: 1,
            adapter: LANE.into(),
            endpoint: endpoint.into(),
        };
        assert_eq!(
            destination_path(&spec("/backup/a.bin", "/x/y.bin")),
            "/backup/a.bin"
        );
        assert_eq!(
            destination_path(&spec("", "/x/y/my clip.mp4")),
            "/my clip.mp4"
        );
        assert_eq!(
            destination_path(&spec("C:\\junk", r"C:\vids\a b.wmv")),
            "/a b.wmv"
        );
        assert_eq!(destination_path(&spec("", "")), "/upload.bin");
    }

    #[test]
    fn the_path_encoder_keeps_slashes_and_encodes_the_rest() {
        assert_eq!(path_encode("backup/my clip.mp4"), "backup/my%20clip.mp4");
        assert_eq!(
            path_encode("видео/1.mp4"),
            "%D0%B2%D0%B8%D0%B4%D0%B5%D0%BE/1.mp4"
        );
        assert_eq!(path_encode("a+b&c=d"), "a%2Bb%26c%3Dd");
    }

    #[test]
    fn the_session_ranges_give_up_their_next_expected_byte() {
        let v = |s: &str| serde_json::Value::String(s.into());
        assert_eq!(next_expected_byte(&[v("5242880-")]), Some(5_242_880));
        assert_eq!(
            next_expected_byte(&[v("0-"), v("7168000-7213055")]),
            Some(0)
        );
        // the min guards against any reordering on the wire
        assert_eq!(
            next_expected_byte(&[v("7168000-7213055"), v("52428800-")]),
            Some(7_168_000)
        );
        assert_eq!(next_expected_byte(&[]), None);
        assert_eq!(next_expected_byte(&[serde_json::Value::Null]), None);
    }

    #[test]
    fn the_classifier_names_the_honest_refusals() {
        let s = |x: u16| StatusCode::from_u16(x).unwrap();
        assert!(matches!(
            classify(s(401), "expired"),
            AdapterError::AuthRequired(_)
        ));
        assert!(matches!(
            classify(s(429), "slow down"),
            AdapterError::Throttled { .. }
        ));
        assert!(matches!(
            classify(s(503), "down"),
            AdapterError::Transient(_)
        ));
        assert!(matches!(
            classify(s(404), r#"{"error":{"code":"itemNotFound"}}"#),
            AdapterError::ExpiredSession(_)
        ));
        assert!(matches!(
            classify(s(410), "gone"),
            AdapterError::ExpiredSession(_)
        ));
        assert!(matches!(
            classify(s(400), r#"{"error":{"code":"invalidRequest"}}"#),
            AdapterError::Unknown(_)
        ));
    }

    #[test]
    fn the_driveitem_hash_extractor_reads_the_official_shape() {
        let item: serde_json::Value = serde_json::from_str(
            r#"{"id":"abc","size":5,"file":{"hashes":{"quickXorHash":"aQeLbCJOPXQTgfRyZzjP7UWb1PM="}}}"#,
        )
        .unwrap();
        assert_eq!(
            quick_xor_from_item(&item),
            Some("aQeLbCJOPXQTgfRyZzjP7UWb1PM=".to_string())
        );
        let bare: serde_json::Value = serde_json::from_str(r#"{"id":"abc","size":5}"#).unwrap();
        assert_eq!(quick_xor_from_item(&bare), None);
    }
}
