//! The Dropbox lane (#109) — IUM's own OAuth app identity, PKCE public
//! client, and the provider's own integrity hash on the verify gate.
//!
//! Unlike the bridge family (#63/#82), Dropbox needs NO client secret: this
//! is a PKCE public client (RFC 7636) registered in IUM's own Dropbox app.
//! The client id is a PUBLIC identifier — the lane's security is the exact
//! redirect match + PKCE, never client secrecy. The refresh token rides the
//! grant (`token_access_type=offline`) and lives in the #81 SecretStore.
//!
//! Wire contract, LIVE-REHEARSED 2026-11-12 against the real API (the #109
//! ledger comment; script `scripts/rehearsal109b.py`):
//! * `upload_session/start → append_v2 → finish(CommitInfo) → FileMetadata`,
//!   the finish answer carries the server's `content_hash`;
//! * a BEHIND-offset append answers 200 and stores NOTHING (idempotent-retry
//!   tolerance) — safe under the same-bytes-per-offset invariant;
//! * an AHEAD-offset append answers **409** `incorrect_offset` whose
//!   `correct_offset` is a SIBLING of `.tag` in the error body;
//! * an empty-body RPC POST must NOT carry `Content-Type: application/json`
//!   (a json content-type on an empty body trips a server-side 500);
//! * content_hash = per-4-MiB-block SHA-256, digests concatenated, SHA-256
//!   of the concatenation, hex — matched Python `hashlib` exactly live.
//!
//! HONEST LIMITS: one sequential-offset session per job (`max_parallel: 1`);
//! per-request payload ≤ 150 MiB (the uniform 4 MiB block is far under);
//! sessions expire after ~7 days — a reattach onto a dead session is an
//! honest `ExpiredSession` and the re-probe restarts the job from zero on
//! the same journal. The non-ASCII destination-path header spelling rides
//! the SDK-standard percent-encoding of `Dropbox-API-Arg` (not exercised in
//! the live rehearsal — the rehearsal path was ASCII; the unit tests pin
//! the encoder).

use std::time::Duration;

use reqwest::header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
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
pub const LANE: &str = "dropbox";
/// IUM's own Dropbox app key — a PUBLIC identifier (PKCE public client; no
/// secret exists on this lane by design).
pub const CLIENT_ID: &str = "dsvpauxp6w4r2ac";
/// The loopback port the redirect comes back to, mirroring the Google lane's
/// fixed-port convention (`google::REDIRECT_PORT = 8737`). Dropbox checks
/// the requested redirect against the REGISTERED values and the port is part
/// of the exact match — one fixed port, registered once.
pub const REDIRECT_PORT: u16 = 8738;
/// The redirect URI, registered exactly in the app console.
pub const REDIRECT_URI: &str = "http://localhost:8738";
/// The prescribed uniform block (#56 channel) = the content-hash block size:
/// chunk boundaries stay on hash-block boundaries so the local verify hash
/// streams block-aligned without cross-chunk carry.
pub const BLOCK: u64 = 4 * 1024 * 1024;
/// The wire's per-request payload cap (official docs; the prescribed block
/// is far under it).
pub const MAX_CHUNK: u64 = 150 * 1024 * 1024;
/// The scope set — FINAL, provider-enforced (the #109 record): Dropbox
/// forces `account_info.read` on every scoped app and auto-includes
/// `files.metadata.read` with any `files.content.*` scope.
pub const SCOPE: &str =
    "account_info.read files.content.read files.content.write files.metadata.read";

const AUTHORIZE: &str = "https://www.dropbox.com/oauth2/authorize";
const TOKEN: &str = "https://api.dropboxapi.com/oauth2/token";
const RPC: &str = "https://api.dropboxapi.com/2";
const CONTENT: &str = "https://content.dropboxapi.com/2";

/// The redirect URI the lane speaks (one spelling, byte-identical on the
/// authorize request and the token exchange — the exact-match rule).
pub fn redirect_uri() -> &'static str {
    REDIRECT_URI
}

/// The official content-hash algorithm (dropbox.com/developers/reference/
/// content-hash): per-4-MiB-block SHA-256, the digests concatenated, SHA-256
/// of the concatenation, hex. Empty input is `sha256("")`. Vectors in the
/// tests were cross-checked against Python `hashlib` (an independent
/// implementation) AND against the live server in the #109 rehearsal.
pub fn content_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut outer = Sha256::new();
    if !data.is_empty() {
        for block in data.chunks(BLOCK as usize) {
            let mut inner = Sha256::new();
            inner.update(block);
            outer.update(inner.finalize());
        }
    }
    hex(&outer.finalize())
}

/// The streaming twin of [`content_hash`] over a file — verify reads the
/// source in 4 MiB blocks and never buffers the whole file. Every block is
/// FILLED before it is hashed (a short read mid-block must never start a
/// new hash block — the block boundaries ARE the algorithm).
async fn file_content_hash(path: &str) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = tokio::fs::File::open(path).await?;
    let mut outer = Sha256::new();
    let mut buf = vec![0u8; BLOCK as usize];
    loop {
        let mut filled = 0usize;
        while filled < buf.len() {
            let n = f.read(&mut buf[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break; // clean EOF on a block boundary
        }
        let mut inner = Sha256::new();
        inner.update(&buf[..filled]);
        outer.update(inner.finalize());
        if filled < buf.len() {
            break; // EOF mid-buffer: that partial block was the last one
        }
    }
    Ok(hex(&outer.finalize()))
}

fn hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Percent-encode everything RFC 3986's unreserved set does not cover —
/// query values and the (rarely needed) non-ASCII `Dropbox-API-Arg` header
/// spelling.
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

/// The consent-screen URL: Dropbox's real page opens in the user's browser,
/// the redirect returns to the loopback listener, and the grant carries a
/// refresh token (`token_access_type=offline`). PKCE S256; the scope set is
/// the fixed 4-scope final state (the #109 record).
pub fn authorize_url(verifier: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type=code&state={}&code_challenge={}&code_challenge_method=S256&token_access_type=offline&scope={}",
        percent_encode(CLIENT_ID),
        percent_encode(REDIRECT_URI),
        percent_encode(state),
        crate::google::pkce_challenge(verifier),
        percent_encode(SCOPE),
    )
}

/// The token client — public client, no secret anywhere.
#[derive(Debug, Clone, Default)]
pub struct DropboxClient;

impl DropboxClient {
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
                .map_err(|e| format!("the token exchange could not reach Dropbox: {e}"))?,
        )
        .await
    }

    /// The refresh grant — the long-lived grant in, a fresh access token
    /// out. Dropbox's short-lived access tokens live ~4 h; the TTL is read
    /// from the answer, never hardcoded.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet, String> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ];
        Self::token_answer(
            Self::http()
                .post(TOKEN)
                .form(&params)
                .send()
                .await
                .map_err(|e| format!("the token refresh could not reach Dropbox: {e}"))?,
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
                "Dropbox refused the token request ({status}): {}",
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
pub struct DropboxConfig {
    /// The stored grant: access token + refresh token + expiry.
    pub token: TokenSet,
}

impl DropboxConfig {
    /// The required-credential check, naming the env var it expected.
    pub fn required(token: Option<&str>) -> Result<String, AdapterError> {
        token
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AdapterError::AuthRequired("IUM_DROPBOX_TOKEN not set".into()))
    }

    /// The pure constructor: a token JSON that is not a token is an honest
    /// refusal (a pasted cookie must never look like a signed-in lane).
    pub fn from_parts(token_json: &str) -> Result<Self, AdapterError> {
        let token = TokenSet::parse(&Self::required(Some(token_json))?).map_err(|e| {
            AdapterError::AuthRequired(format!(
                "the Dropbox sign-in token is unusable — connect once again. ({e})"
            ))
        })?;
        Ok(Self { token })
    }

    /// The env constructor (`IUM_DROPBOX_TOKEN`; the vault's apply_env fills
    /// it from the sign-in's stored grant).
    pub fn from_env() -> Result<Self, AdapterError> {
        Self::from_parts(&std::env::var("IUM_DROPBOX_TOKEN").unwrap_or_default())
    }
}

/// The session truth, mutable across confirms and reattaches.
#[derive(Default)]
struct SessionState {
    /// The Dropbox session id (the journaled probe token's living form).
    session_id: Option<String>,
    /// The committed answer (path + server content_hash) finish carried.
    committed: Option<(String, String)>,
    /// The access token the lane rides NOW (refreshed lazily on expiry).
    access_token: String,
    /// Its absolute expiry (unix seconds); the refresh fires 60 s early.
    access_expires_at: Option<u64>,
}

/// The Dropbox adapter — one connected account, one lane.
pub struct DropboxAdapter {
    cfg: DropboxConfig,
    http: Client,
    state: std::sync::RwLock<SessionState>,
}

fn req_err(e: reqwest::Error) -> AdapterError {
    AdapterError::Transient(format!("transport: {e}"))
}

fn io_err(e: std::io::Error) -> AdapterError {
    AdapterError::Transient(format!("local io: {e}"))
}

/// The destination path inside the app folder. The create form's endpoint
/// (an absolute app-folder path starting with `/`) wins; otherwise the
/// source's file name lands at the app-folder root — the same filename
/// discipline the YouTube lane's title rides.
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

/// A wire error's classification — the tags the lane actually meets,
/// including the rehearsal's flattened `incorrect_offset` shape.
fn classify(status: StatusCode, body: &str) -> AdapterError {
    if status == StatusCode::UNAUTHORIZED {
        return AdapterError::AuthRequired(format!(
            "Dropbox refused the credential ({status}) — connect once again: {}",
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
            "Dropbox answered {status}: {}",
            body.chars().take(160).collect::<String>()
        ));
    }
    // Session lifecycle tags: a lookup failure on the cursor means the
    // session is gone (the ~7-day expiry) — the honest restart path.
    if body.contains("not_found") || body.contains("closed") {
        return AdapterError::ExpiredSession(format!(
            "the upload session is gone from Dropbox ({status}): {}",
            body.chars().take(160).collect::<String>()
        ));
    }
    AdapterError::Unknown(format!(
        "Dropbox answered {status}: {}",
        body.chars().take(200).collect::<String>()
    ))
}

/// Pull the `correct_offset` out of an error body, accepting BOTH shapes:
/// the live flattened one (`error.{".tag": "incorrect_offset",
/// "correct_offset": N}`) and the stone-nested one
/// (`error.lookup_failed.incorrect_offset.correct_offset`).
fn correct_offset_from(body: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    fn walk(v: &serde_json::Value) -> Option<u64> {
        let obj = v.as_object()?;
        if obj.get(".tag").and_then(|t| t.as_str()) == Some("incorrect_offset") {
            return obj.get("correct_offset").and_then(|c| c.as_u64());
        }
        if let Some(nested) = obj.get("incorrect_offset") {
            return nested.get("correct_offset").and_then(|c| c.as_u64());
        }
        for (_, child) in obj {
            if let Some(found) = walk(child) {
                return Some(found);
            }
        }
        None
    }
    walk(v.get("error")?)
}

/// The wire-byte counter over a seeked source — the shared adapter pattern
/// (S3, HTTP, YouTube ride the same shape).
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

impl DropboxAdapter {
    /// Production constructor: the config from env, fresh token carried.
    pub fn new(cfg: DropboxConfig) -> Self {
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
    /// original — a restart re-reads it. Dropbox may rotate the refresh
    /// token on use; the in-process answer wins until the next restart,
    /// the same honest scope the YouTube lane documents).
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
    /// refusal on exhaustion.
    async fn force_refresh(&self) -> Result<String, AdapterError> {
        let Some(rt) = self.cfg.token.refresh_token.clone() else {
            return Err(AdapterError::AuthRequired(
                "the Dropbox sign-in expired and carries no refresh token — connect once again"
                    .into(),
            ));
        };
        let fresh = DropboxClient
            .refresh(&rt)
            .await
            .map_err(AdapterError::AuthRequired)?;
        let access = fresh.access_token.clone();
        let exp = fresh.expires_at;
        let mut st = self.state.write().expect("state lock");
        st.access_token = access.clone();
        st.access_expires_at = exp;
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

    /// One RPC call (api.dropboxapi.com/2/...) with the 401-refresh-once
    /// dance. An empty-body RPC must NOT carry a json content-type (the
    /// live 500 lesson): the body decides the headers.
    async fn rpc(
        &self,
        route: &str,
        json: Option<serde_json::Value>,
    ) -> Result<Result<serde_json::Value, (StatusCode, String)>, AdapterError> {
        for attempt in 0..2 {
            let token = if attempt == 0 {
                self.bearer().await?
            } else {
                self.force_refresh().await?
            };
            let mut req = self.http.post(format!("{RPC}/{route}")).bearer_auth(&token);
            if let Some(v) = &json {
                req = req
                    .header(CONTENT_TYPE, "application/json")
                    .body(v.to_string());
            }
            let resp = req.send().await.map_err(req_err)?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                continue; // one forced-refresh retry, then the honest refusal
            }
            if status.is_success() {
                let parsed = if body.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(&body).map_err(|e| {
                        AdapterError::Unknown(format!("the {route} answer is not JSON: {e}"))
                    })?
                };
                return Ok(Ok(parsed));
            }
            return Ok(Err((status, body)));
        }
        unreachable!("the loop returns on every branch")
    }

    /// One content WIRE ATTEMPT (content.dropboxapi.com/2/...) with the
    /// `Dropbox-API-Arg` header — the upload session's whole vocabulary
    /// (start / append_v2 / finish) rides this. The arg header is
    /// percent-encoded when it carries non-ASCII bytes (the SDK-standard
    /// spelling for unicode paths), raw when ASCII (the rehearsed shape).
    /// ONE attempt, token passed in: a streaming body cannot be cloned, so
    /// the 401-refresh and realign budgets live in the CALLERS, which
    /// rebuild the body per attempt (the same-bytes-per-offset invariant
    /// makes every rebuild byte-identical).
    async fn content_once(
        &self,
        token: &str,
        route: &str,
        api_arg: &serde_json::Value,
        body: reqwest::Body,
        content_length: u64,
    ) -> Result<(StatusCode, String), AdapterError> {
        let value = HeaderValue::from_str(&api_arg_header(api_arg))
            .map_err(|e| AdapterError::Unknown(format!("the api-arg header refused: {e}")))?;
        let resp = self
            .http
            .post(format!("{CONTENT}/{route}"))
            .bearer_auth(token)
            .header("Dropbox-API-Arg", value)
            .header(CONTENT_TYPE, "application/octet-stream")
            .header(
                CONTENT_LENGTH,
                HeaderValue::from_str(&content_length.to_string())
                    .expect("a decimal length is always a header value"),
            )
            .body(body)
            .send()
            .await
            .map_err(req_err)?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Ok((status, text))
    }

    /// The read-only credential check (#81): `get_current_account` — the
    /// metadata read the Test button drives, never an upload session it
    /// could orphan.
    pub async fn check_credentials(&self) -> Result<(), AdapterError> {
        match self.rpc("users/get_current_account", None).await? {
            Ok(_) => Ok(()),
            Err((status, body)) => Err(classify(status, &body)),
        }
    }

    /// Open the upload session — the probe event's journaled token.
    async fn start_session(&self) -> Result<String, AdapterError> {
        let arg = serde_json::json!({"close": false});
        let mut token = self.bearer().await?;
        let answer = loop {
            let (status, body) = self
                .content_once(
                    &token,
                    "files/upload_session/start",
                    &arg,
                    reqwest::Body::from(""),
                    0,
                )
                .await?;
            if status == StatusCode::UNAUTHORIZED {
                token = self.force_refresh().await?;
                continue; // the empty body rebuilds for free — one retry
            }
            break (status, body);
        };
        let (status, body) = answer;
        match status.is_success() {
            true => {
                let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                    AdapterError::Unknown(format!("the start answer is not JSON: {e}"))
                })?;
                let id = v
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        AdapterError::Unknown("the start answer carries no session_id".into())
                    })?;
                Ok(id)
            }
            false => Err(classify(status, &body)),
        }
    }

    /// Append one span at an absolute offset, realigning on the wire's
    /// 409 (bounded — a realign that lands outside the lease is honest
    /// corruption, never a silent improvisation).
    async fn append_span(
        &self,
        session_id: &str,
        spec: &JobSpec,
        from: u64,
        end: u64,
        progress: &ProgressReporter,
    ) -> Result<(), AdapterError> {
        let mut from = from;
        let mut token = self.bearer().await?;
        let mut refreshed = false;
        let mut realigns = 0u32;
        loop {
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
            let arg = serde_json::json!({
                "cursor": {"session_id": session_id, "offset": from},
                "close": false,
            });
            let (status, body) = self
                .content_once(
                    &token,
                    "files/upload_session/append_v2",
                    &arg,
                    reqwest::Body::wrap_stream(stream),
                    span,
                )
                .await?;
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                // The stream is rebuilt byte-identical on the next iteration
                // (the invariant), so the retry is honest.
                refreshed = true;
                token = self.force_refresh().await?;
                continue;
            }
            if status.is_success() {
                // 200: the server holds [from, end) — a fresh append, a
                // deduped retry tail, or the silent no-op of a range it
                // already stored (the rehearsed behind-offset semantics;
                // the same-bytes-per-offset invariant makes all three
                // land in the same place, and verify is the net).
                return Ok(());
            }
            let Some(correct) = correct_offset_from(&body) else {
                return Err(classify(status, &body));
            };
            if correct >= end {
                continue; // the server already holds our whole span
            }
            if correct <= from {
                // The server is BEHIND our lease start: bytes before
                // this lease are missing — the journal and the wire
                // diverged. Honest corruption, never a re-send of
                // another chunk's range behind the manager's back.
                return Err(AdapterError::Corruption(format!(
                    "the session holds {correct} bytes but this chunk starts at {from} — the journal is ahead of the wire"
                )));
            }
            realigns += 1;
            if realigns > 2 {
                return Err(AdapterError::Corruption(
                    "the session kept correcting its offset beyond the realign budget".into(),
                ));
            }
            from = correct; // server wins realign; the loop re-sends the tail
        }
    }
}

#[async_trait::async_trait]
impl TransferAdapter for DropboxAdapter {
    fn name(&self) -> &'static str {
        LANE
    }

    fn capabilities(&self) -> Capability {
        // Session-based resume; NO dynamic chunks (the prescribed 4 MiB
        // geometry owns the plan) and NO tail-race (append_v2 is one
        // strictly sequential-offset resource).
        Capability::new(Capability::RESUMABLE)
    }

    fn uniform_block(&self) -> Option<u64> {
        Some(BLOCK)
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.source_len(spec).await?;
        let session_id = self.start_session().await?;
        {
            let mut st = self.state.write().expect("state lock");
            st.session_id = Some(session_id.clone());
            st.committed = None;
        }
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(MAX_CHUNK),
            min_chunk_bytes: None,
            max_parallel: Some(1),
            session_token: Some(session_id),
            uniform_block: Some(BLOCK),
            concatenation: false,
        })
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        self.source_len(spec).await?;
        let session_id = self
            .state
            .read()
            .expect("state lock")
            .session_id
            .clone()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no live session in this process — reattach from the journaled token".into(),
                )
            })?;
        if lease.resume_from >= lease.end {
            // The server already holds this whole segment (a prior process
            // confirmed it and died before journalling).
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: 0,
                adapter_token: None,
            });
        }
        self.append_span(&session_id, spec, lease.resume_from, lease.end, progress)
            .await?;
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: lease.end - lease.resume_from,
            adapter_token: None,
        })
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let path = match self.state.read().expect("state lock").committed.clone() {
            Some((p, _)) => p,
            None => destination_path(spec),
        };
        let meta = match self
            .rpc(
                "files/get_metadata",
                Some(serde_json::json!({"path": path})),
            )
            .await?
        {
            Ok(v) => v,
            Err((status, body)) => {
                return Ok(VerifyReport {
                    ok: false,
                    detail: Some(format!(
                        "the verify read of {path} failed ({status}): {}",
                        body.chars().take(160).collect::<String>()
                    )),
                });
            }
        };
        let server_hash = meta
            .get("content_hash")
            .and_then(|h| h.as_str())
            .map(str::to_string);
        let server_size = meta.get("size").and_then(|s| s.as_u64());
        let Some(server_hash) = server_hash else {
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "the metadata for {path} carries no content_hash — verify refuses to declare"
                )),
            });
        };
        let local = file_content_hash(&spec.source_path).await.map_err(io_err)?;
        let size_ok = server_size.map(|s| s == spec.source_size).unwrap_or(false);
        let ok = server_hash == local && size_ok;
        Ok(VerifyReport {
            ok,
            detail: Some(if ok {
                format!(
                    "Dropbox content_hash {server_hash} matches the local computation over {} bytes",
                    spec.source_size
                )
            } else {
                format!(
                    "content_hash mismatch — server {server_hash}, local {local}, server size {server_size:?}, journal size {}",
                    spec.source_size
                )
            }),
        })
    }

    async fn reattach(&self, _endpoint: &str, token: &str) -> Result<(), AdapterError> {
        // The journaled token IS the session id. Never CREATE a session —
        // only adopt the journaled one; the offsets live in the journal.
        {
            let mut st = self.state.write().expect("state lock");
            st.session_id = Some(token.to_string());
            st.committed = None;
        }
        Ok(())
    }

    async fn finalize(&self, spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        let total = self.source_len(spec).await?;
        let session_id = self
            .state
            .read()
            .expect("state lock")
            .session_id
            .clone()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no live session in this process — reattach from the journaled token".into(),
                )
            })?;
        let dest = destination_path(spec);
        let offset = total;
        let mut token = self.bearer().await?;
        let mut refreshed = false;
        loop {
            let arg = serde_json::json!({
                "cursor": {"session_id": session_id, "offset": offset},
                "commit": {"path": dest, "mode": "overwrite", "autorename": false},
            });
            let (status, body) = self
                .content_once(
                    &token,
                    "files/upload_session/finish",
                    &arg,
                    reqwest::Body::from(""),
                    0,
                )
                .await?;
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                token = self.force_refresh().await?;
                continue;
            }
            if status.is_success() {
                let meta: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                    AdapterError::Unknown(format!("the finish answer is not JSON: {e}"))
                })?;
                let path = meta
                    .get("path_lower")
                    .or_else(|| meta.get("path_display"))
                    .and_then(|p| p.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| dest.clone());
                let hash = meta
                    .get("content_hash")
                    .and_then(|h| h.as_str())
                    .map(str::to_string);
                {
                    let mut st = self.state.write().expect("state lock");
                    st.committed = Some((path.clone(), hash.unwrap_or_default()));
                }
                return Ok(Some(path));
            }
            let Some(correct) = correct_offset_from(&body) else {
                return Err(classify(status, &body));
            };
            if correct == offset {
                continue; // the same cursor answered incorrect once — one retry
            }
            // The server's cursor disagrees with the journal's completed
            // byte count — divergence, not a resume.
            return Err(AdapterError::Corruption(format!(
                "the session holds {correct} bytes but the journal completed {total}"
            )));
        }
    }
}

/// The `Dropbox-API-Arg` header spelling: raw JSON when ASCII (the
/// rehearsed shape), percent-encoded when it carries non-ASCII bytes —
/// the SDK-standard fallback for unicode commit paths.
fn api_arg_header(value: &serde_json::Value) -> String {
    let arg = value.to_string();
    if arg.is_ascii() {
        arg
    } else {
        percent_encode(&arg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lane_pins_its_wire_constants() {
        assert_eq!(LANE, "dropbox");
        assert_eq!(CLIENT_ID, "dsvpauxp6w4r2ac");
        assert_eq!(REDIRECT_PORT, 8738);
        assert_eq!(redirect_uri(), "http://localhost:8738");
        assert_eq!(BLOCK, 4 * 1024 * 1024);
        assert_eq!(MAX_CHUNK, 150 * 1024 * 1024);
        assert_eq!(
            SCOPE,
            "account_info.read files.content.read files.content.write files.metadata.read"
        );
    }

    #[test]
    fn content_hash_matches_the_official_vectors() {
        // Vectors cross-checked with Python hashlib (scripts/gen_hash_vectors.py).
        assert_eq!(
            content_hash(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let pattern = |n: usize| (0..n).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
        assert_eq!(
            content_hash(&pattern(1024 * 1024)),
            "004fbf44b4c0a58632164fc9ec00e8f35e330037fd4425dd947cf65d930762c2"
        );
        assert_eq!(
            content_hash(&pattern(4 * 1024 * 1024)),
            "b9654428408015906b44a00935b70af33830aa344b780b0eabd535a133150d04"
        );
        assert_eq!(
            content_hash(&pattern(5 * 1024 * 1024)),
            "b109747a45702e22475f0ae8a9056c7621fac74aad54f7bf80bdc6bb157d897f"
        );
        assert_eq!(
            content_hash(&pattern(2 * 4 * 1024 * 1024 + 1024 * 1024 + 1)),
            "040a6fa5f007f1f3df8bd13fec85bcbfc26fe3a288b75c0e2f0bad4b9ce279a2"
        );
    }

    #[test]
    fn the_authorize_url_carries_pkce_offline_and_the_exact_redirect() {
        let url = authorize_url("verifier-abc", "state-xyz");
        assert!(url.starts_with("https://www.dropbox.com/oauth2/authorize?"));
        assert!(url.contains("client_id=dsvpauxp6w4r2ac"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8738"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("token_access_type=offline"));
        assert!(url.contains("state=state-xyz"));
        assert!(url.contains("scope=account_info.read%20files.content"));
    }

    #[test]
    fn the_config_refuses_a_missing_or_garbage_token_by_name() {
        let e = DropboxConfig::required(None).unwrap_err();
        assert!(e.to_string().contains("IUM_DROPBOX_TOKEN"));
        let e = DropboxConfig::from_parts("not-json").unwrap_err();
        assert!(matches!(e, AdapterError::AuthRequired(_)));
        DropboxConfig::from_parts(r#"{"access_token":"a","refresh_token":"r","expires_in":14400}"#)
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
    fn the_rehearsed_error_shapes_give_up_their_correct_offset() {
        // The LIVE shape (2026-11-12): .tag and correct_offset are siblings.
        let live = r#"{"error": {".tag": "incorrect_offset", "correct_offset": 4194304},
                       "error_summary": "incorrect_offset/"}"#;
        assert_eq!(correct_offset_from(live), Some(4_194_304));
        // The stone-nested shape, defensively.
        let nested = r#"{"error": {"lookup_failed": {".tag": "incorrect_offset",
                       "correct_offset": 8388608}}}"#;
        assert_eq!(correct_offset_from(nested), Some(8_388_608));
        assert_eq!(correct_offset_from(r#"{"error": {".tag": "reset"}}"#), None);
        assert_eq!(correct_offset_from("not json"), None);
    }

    #[test]
    fn the_api_arg_header_stays_raw_for_ascii_and_encodes_unicode() {
        let ascii = serde_json::json!({"cursor": {"session_id": "pid_up:abc", "offset": 8}});
        let v = api_arg_header(&ascii);
        assert!(v.contains("\"offset\":8"));
        let uni = serde_json::json!({"commit": {"path": "/видео 1.mp4"}});
        let v = api_arg_header(&uni);
        assert!(v.starts_with("%7B%22commit"));
    }

    #[tokio::test]
    async fn the_file_hash_streaming_twin_matches_the_in_memory_one() {
        let dir = std::env::temp_dir().join(format!("ium-dropbox-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("block.bin");
        let data: Vec<u8> = (0..(BLOCK as usize + 1234))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&path, &data).unwrap();
        let streamed = file_content_hash(path.to_str().unwrap()).await.unwrap();
        assert_eq!(streamed, content_hash(&data));
        std::fs::remove_dir_all(&dir).ok();
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
            classify(s(409), r#"{"error":{".tag":"not_found"}}"#),
            AdapterError::ExpiredSession(_)
        ));
        assert!(matches!(
            classify(s(400), r#"{"error":{".tag":"path"}}"#),
            AdapterError::Unknown(_)
        ));
    }
}
