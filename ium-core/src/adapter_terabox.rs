//! The TeraBox lane (issue #64, M10) — the community web protocol as a
//! first-class [`TransferAdapter`].
//!
//! # Wire facts (rehearsed LIVE 2026-11-06 against the real `dm.terabox.com`
//! deployment — 2 MiB single-slice AND 9 MiB 3-slice roundtrips BYTE-EXACT;
//! probes at `scripts/wirefacts64/`)
//!
//! * **W1** the API surface is **`dm.terabox.com`** — `www` answers errno -6
//!   ("user not login") with the SAME cookie jar; the regional hosts
//!   (`c-jp`/`data`/`up`) answer 31045 "user not exists". Auth = the **`ndus`
//!   session cookie** (the one load-bearing credential) + a browser UA.
//! * **W3** **jsToken is self-serve**: embedded in the logged-in `/main` HTML
//!   (`fn("<128-hex>")`, URL-encoded or raw). Without it every WRITE endpoint
//!   answers errno 4000023 "need verify" (risk control); with it they pass.
//!   The adapter fetches it ITSELF at probe — no owner Network-tab step.
//! * **W4** `POST /api/precreate` (form: `path`, `autoinit=1`, `target_path`,
//!   `block_list` = JSON array of per-slice MD5s, `size`) → `uploadid`
//!   (session-bound; embeds the client IP). Without `autoinit=1` there is NO
//!   uploadid (return_type 1, dead end — probed).
//! * **W5** `GET /rest/2.0/pcs/file?method=locateupload` assigns the
//!   per-session upload host (`dm1-cdata.terabox.com`…), **expire 600 s**.
//!   The host is EPHEMERAL routing — never journal truth; refreshed at 90%
//!   life (the Feishu token-cache pattern).
//! * **W6** `POST https://{uhost}/rest/2.0/pcs/superfile2` — multipart-form,
//!   binary field named **`file`**, query `path`, `uploadid`, `uploadsign=0`,
//!   `partseq=i` → 200 with **the part's MD5 echoed back** — matched the
//!   local MD5 exactly (a built-in integrity gate: echo mismatch →
//!   [`AdapterError::Corruption`]).
//! * **W7** `POST /api/create` (form: `path`, `size`, `uploadid`,
//!   `block_list`, `target_path`, `local_mtime`, `isdir=0`, `rtype=1`) →
//!   `fs_id`; `POST /api/filemanager?opera=delete` removes the file (drill
//!   cleanup; errno 0).
//! * **W8** **verify route = PCS `method=download`** on dm.terabox.com →
//!   streamed bytes (302 to a signed CDN URL in the first rehearsal; direct
//!   200 on the 3-slice rehearsal). `/api/download` answers errno 2 and the
//!   `filemetas` dlink is UNRELIABLE on multi-slice files (**W13**) — dead
//!   ends recorded so nobody re-tries them.
//! * **W10** **multi-slice PROVEN**: 3-entry `block_list` (4+4+1 MiB) →
//!   errno 0; the 9 MiB roundtrip byte-exact. The lane PRESCRIBES uniform
//!   4 MiB slices + exact tail (the #56 machinery: chunk *i* ↔ partseq *i* ↔
//!   `block_list[*i*]`).
//! * **W11** **out-of-order partseq ACCEPTED** (partseq 1 before 0 → 200).
//! * **W12** **re-PUT of a done partseq is IDEMPOTENT** (200, same MD5 echo)
//!   — crash-window re-sends are safe; resume re-uploads only unconfirmed
//!   parts (journal truth, ADR-002).
//!
//! # Design (the design note on #64, D1–D6)
//!
//! * **D1** geometry by prescription: [`TERABOX_BLOCK`] uniform + exact tail,
//!   declared at create time ([`TransferAdapter::uniform_block`]) and probe
//!   time ([`ProbeReport::uniform_block`]); serial lane
//!   (`max_parallel = Some(1)` — no docs on concurrent partseq, risk control
//!   exists).
//! * **D2** session = the `uploadid` (+ remote path). Parts are atomic; the
//!   confirm credits the full span exactly once the server took the part.
//!   Per-slice MD5s are RECOMPUTED per call (deterministic over the
//!   journal-immutable source) instead of bloating the journaled token.
//! * **D3** finalize = `create` → `fs_id` through the manager's finalize-once
//!   machinery (the #50 crash-window repair rides free — the token is the
//!   JSON shape). Verify = streamed download + sha256 vs source (W8).
//! * **D4** error mapping is loud, only proven codes: errno -6 →
//!   [`AdapterError::AuthRequired`] (ndus dead — the owner re-extracts
//!   cookies), 4000023 → [`AdapterError::AuthRequired`] (risk control),
//!   errno 2 in download context → [`AdapterError::Corruption`], HTTP 429 →
//!   [`AdapterError::Throttled`], 5xx/connect → [`AdapterError::Transient`],
//!   everything else → [`AdapterError::Unknown`] (never guessed).
//! * **D5** credentials from env (`IUM_TERABOX_NDUS`, optional
//!   `IUM_TERABOX_APP_ID` default `250528`, `IUM_TERABOX_BASE` for loopback
//!   tests); the live drill is env-gated and skips loud without them — no
//!   secrets ever in the repo, journal or logs.

use async_trait::async_trait;
use md5::{Digest as Md5Digest, Md5};
use serde_json::json;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::sigv4::uri_encode;
use crate::JobSpec;

/// The prescribed TeraBox slice size (W10: proven 4+4+1 MiB on the live wire;
/// the community convention for `block_list`).
pub const TERABOX_BLOCK: u64 = 4 * 1024 * 1024;

const DEFAULT_BASE: &str = "https://dm.terabox.com";
const DEFAULT_APP_ID: &str = "250528";
/// A browser UA is load-bearing on this wire (the probes' exact shape).
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// TeraBox credentials (D5): the `ndus` session cookie is the only
/// load-bearing value; jsToken is NEVER config (W3 self-serve).
#[derive(Debug, Clone)]
pub struct TeraboxConfig {
    /// The `ndus` cookie value (never journaled, never logged — env-injected).
    pub ndus: String,
    /// The community app id (default `250528`, the probes' proven value).
    pub app_id: String,
    /// The API base (default `https://dm.terabox.com`; loopback tests inject
    /// their own — W1: regional deployments differ).
    pub base: String,
}

impl TeraboxConfig {
    /// Build from explicit credentials (tests, embedders).
    pub fn new(ndus: String, app_id: String) -> Self {
        Self {
            ndus,
            app_id,
            base: DEFAULT_BASE.to_string(),
        }
    }

    /// Read from the environment (`IUM_TERABOX_NDUS`; optional
    /// `IUM_TERABOX_APP_ID` / `IUM_TERABOX_BASE`) — the env-gated drill lane
    /// (D5). Missing ndus = [`AdapterError::AuthRequired`]: user action
    /// required (extract the cookie from a logged-in browser session).
    pub fn from_env() -> Result<Self, AdapterError> {
        let ndus = std::env::var("IUM_TERABOX_NDUS")
            .map_err(|_| AdapterError::AuthRequired("IUM_TERABOX_NDUS not set".into()))?;
        let app_id =
            std::env::var("IUM_TERABOX_APP_ID").unwrap_or_else(|_| DEFAULT_APP_ID.to_string());
        let base = std::env::var("IUM_TERABOX_BASE").unwrap_or_else(|_| DEFAULT_BASE.to_string());
        Ok(Self { ndus, app_id, base })
    }
}

/// One upload session per endpoint (the in-memory mirror of the journaled
/// probe token — restored by `reattach`, never re-probed). `block_md5s` are
/// deliberately NOT here (D2: recomputed per call over the journal-immutable
/// source; the token stays small).
#[derive(Debug, Clone)]
struct TeraboxSession {
    uploadid: String,
    /// The full remote path (`{endpoint dir}/{file name}`).
    path: String,
    /// The assigned upload host (W5, ephemeral; refreshed at 90% of 600 s).
    uhost: Option<String>,
    uhost_at: Option<Instant>,
    /// Set by finalize (`fs_id`) — the finalize-once repair restores it.
    fs_id: Option<String>,
}

/// The adapter. One `Mutex`-guarded session map keyed by the job endpoint
/// (the Feishu/tus pattern): probe inserts, `reattach` restores from the
/// journal.
pub struct TeraboxAdapter {
    http: reqwest::Client,
    cfg: TeraboxConfig,
    js_token: tokio::sync::Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, TeraboxSession>>,
}

impl TeraboxAdapter {
    /// Production constructor against the real deployment (W1 base).
    pub fn new(cfg: TeraboxConfig) -> Self {
        let base = cfg.base.clone();
        Self::with_base(cfg, base)
    }

    /// Constructor with an explicit base URL (loopback tests).
    pub fn with_base(cfg: TeraboxConfig, base: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .expect("reqwest client builds"),
            cfg: TeraboxConfig { base, ..cfg },
            js_token: tokio::sync::Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The journaled session token (D2): JSON so the manager's finalize-once
    /// machinery recognizes the shape and its crash-window repair can inject
    /// `final` = fs_id before reattach.
    pub fn session_token_json(uploadid: &str, path: &str, fs_id: Option<&str>) -> String {
        json!({
            "uploadid": uploadid,
            "path": path,
            "final": fs_id,
        })
        .to_string()
    }

    /// Inverse of [`TeraboxAdapter::session_token_json`]; `None` for any
    /// non-JSON token (a plain string is not this lane's shape).
    pub fn parse_session_token(token: &str) -> Option<(String, String, Option<String>)> {
        let v: serde_json::Value = serde_json::from_str(token).ok()?;
        let uploadid = v.get("uploadid")?.as_str()?.to_string();
        let path = v.get("path")?.as_str()?.to_string();
        let fs_id = v.get("final").and_then(|f| f.as_str()).map(String::from);
        Some((uploadid, path, fs_id))
    }

    /// Wire errno → retry-taxonomy mapping (D4). Loud, never guessed:
    /// -6 → AuthRequired (the ndus cookie is dead — docs/09 §7's known
    /// fragility), 4000023 → AuthRequired (risk control: jsToken missing or
    /// rejected), errno 2 in the download context → Corruption (the file is
    /// not at the destination), everything else → Unknown.
    pub fn map_errno(errno: i64, msg: &str, context: &str) -> AdapterError {
        match errno {
            -6 => AdapterError::AuthRequired(format!(
                "terabox errno -6 not login ({context}): {msg} — the ndus cookie expired; \
                 re-extract it from a logged-in session"
            )),
            4000023 => AdapterError::AuthRequired(format!(
                "terabox errno 4000023 need verify ({context}): {msg} — jsToken rejected \
                 by risk control"
            )),
            2 if context == "download" => AdapterError::Corruption(format!(
                "terabox errno 2 file missing at destination ({context}): {msg}"
            )),
            other => AdapterError::Unknown(format!("terabox errno {other} ({context}): {msg}")),
        }
    }

    /// HTTP status → taxonomy mapping for the binary/stream endpoints
    /// (superfile2, download) where the envelope is not a JSON errno.
    fn map_status(status: u16, context: &str) -> AdapterError {
        match status {
            429 => AdapterError::Throttled {
                retry_after_ms: None,
                message: format!("terabox {context}: HTTP 429"),
            },
            500..=599 => AdapterError::Transient(format!("terabox {context}: HTTP {status}")),
            401 | 403 => AdapterError::AuthRequired(format!("terabox {context}: HTTP {status}")),
            other => AdapterError::Unknown(format!("terabox {context}: HTTP {other}")),
        }
    }

    /// Extract the jsToken from the `/main` HTML (W3): `fn("<hex>")`, either
    /// URL-encoded or raw. `None` when absent (expired cookies, deployment
    /// change) — the caller fails loud.
    pub fn extract_js_token(html: &str) -> Option<String> {
        for (needle, closer) in [("fn%28%22", "%22%29"), ("fn(\"", "\")")] {
            if let Some(i) = html.find(needle) {
                let rest = &html[i + needle.len()..];
                if let Some(j) = rest.find(closer) {
                    let candidate = &rest[..j];
                    if candidate.len() >= 32 && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Some(candidate.to_string());
                    }
                }
            }
        }
        None
    }

    fn cookie(&self) -> String {
        format!("lang=en; ndus={}", self.cfg.ndus)
    }

    /// A fresh dp-logid (the probes' random-hex shape; uniqueness is all the
    /// wire needs).
    fn logid() -> String {
        crate::ids::new_id().replace('-', "")
    }

    /// The common query prefix every authenticated API call rides on
    /// (the probes' exact shape and order).
    fn api_query(&self, jt: &str) -> String {
        format!(
            "app_id={}&web=1&channel=dubox&clienttype=0&jsToken={jt}&dp-logid={}",
            self.cfg.app_id,
            Self::logid()
        )
    }

    /// The cached jsToken (W3): fetched once from `{base}/main` with the same
    /// ndus cookie; a 4000023 mid-lane stays LOUD (D4) — no silent refetch of
    /// a risk-control surface.
    async fn js_token(&self) -> Result<String, AdapterError> {
        {
            let guard = self.js_token.lock().await;
            if let Some(t) = guard.as_ref() {
                return Ok(t.clone());
            }
        }
        let resp = self
            .http
            .get(format!("{}/main", self.cfg.base))
            .header("Cookie", self.cookie())
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("terabox /main: {e}")))?;
        if !resp.status().is_success() {
            return Err(Self::map_status(
                resp.status().as_u16(),
                "/main (jsToken self-serve)",
            ));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| AdapterError::Transient(format!("terabox /main body: {e}")))?;
        let t = Self::extract_js_token(&html).ok_or_else(|| {
            AdapterError::AuthRequired(
                "terabox /main carries no jsToken — the ndus cookie expired or the \
                 deployment changed; re-extract the cookie"
                    .into(),
            )
        })?;
        *self.js_token.lock().await = Some(t.clone());
        Ok(t)
    }

    /// One authenticated form POST to an `/api/*` endpoint → the parsed JSON
    /// envelope (the errno field is the real verdict).
    async fn api_form(
        &self,
        path: &str,
        query: &str,
        fields: &[(&str, String)],
    ) -> Result<serde_json::Value, AdapterError> {
        let resp = self
            .http
            .post(format!("{}{}?{}", self.cfg.base, path, query))
            .header("Cookie", self.cookie())
            .header("User-Agent", USER_AGENT)
            .header("Referer", format!("{}/main", self.cfg.base))
            .form(fields)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("terabox {path}: {e}")))?;
        let status = resp.status();
        let raw = resp
            .bytes()
            .await
            .map_err(|e| AdapterError::Transient(format!("terabox {path} body: {e}")))?;
        let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| {
            AdapterError::Unknown(format!(
                "terabox {path}: non-JSON envelope, HTTP {} ({} bytes)",
                status,
                raw.len()
            ))
        })?;
        let errno = v.get("errno").and_then(|e| e.as_i64()).unwrap_or(0);
        if errno != 0 {
            let msg = v
                .get("errmsg")
                .and_then(|m| m.as_str())
                .unwrap_or("?")
                .to_string();
            return Err(Self::map_errno(
                errno,
                &msg,
                path.trim_start_matches("/api/"),
            ));
        }
        Ok(v)
    }

    /// The W5 host assignment (a READ — legal in reattach; it never creates
    /// a session).
    async fn locate_upload_host(&self, jt: &str) -> Result<String, AdapterError> {
        let v = self
            .api_form(
                "/rest/2.0/pcs/file",
                &format!("method=locateupload&{}", self.api_query(jt)),
                &[],
            )
            .await?;
        v.get("server")
            .and_then(|s| s.as_array())
            .and_then(|a| a.first())
            .and_then(|s| s.as_str())
            .map(String::from)
            .ok_or_else(|| {
                AdapterError::Unknown("terabox locateupload without a server list".into())
            })
    }

    fn session(&self, endpoint: &str) -> Option<TeraboxSession> {
        self.sessions.lock().unwrap().get(endpoint).cloned()
    }

    fn insert_session(&self, endpoint: &str, s: TeraboxSession) {
        self.sessions
            .lock()
            .unwrap()
            .insert(endpoint.to_string(), s);
    }

    /// The MD5 sweep (D2): whole-file MD5 + one MD5 per [`TERABOX_BLOCK`]
    /// slice, ONE streaming pass over the source. Returns
    /// `(whole_md5, slice_md5s)`.
    async fn md5_sweep(path: &str) -> Result<(String, Vec<String>), AdapterError> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| AdapterError::Corruption(format!("source unreadable: {e}")))?;
        let mut whole = Md5::new();
        let mut block = Md5::new();
        let mut slices = Vec::new();
        let mut in_block = 0u64;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| AdapterError::Corruption(format!("source read: {e}")))?;
            if n == 0 {
                break;
            }
            let mut rest = &buf[..n];
            whole.update(rest);
            while !rest.is_empty() {
                let take = (TERABOX_BLOCK - in_block).min(rest.len() as u64) as usize;
                block.update(&rest[..take]);
                in_block += take as u64;
                rest = &rest[take..];
                if in_block == TERABOX_BLOCK {
                    slices.push(hex_md5(&block.finalize()));
                    block = Md5::new();
                    in_block = 0;
                }
            }
        }
        if in_block > 0 {
            slices.push(hex_md5(&block.finalize()));
        }
        Ok((hex_md5(&whole.finalize()), slices))
    }

    /// The multipart body for `superfile2` (W6): one binary part named
    /// **`file`** — the probes' exact shape (all parameters ride the query).
    pub(crate) fn build_multipart(file_bytes: &[u8], file_name: &str, boundary: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(file_bytes.len() + 256);
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
                 filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        out.extend_from_slice(file_bytes);
        out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        out
    }

    fn boundary() -> String {
        format!("ium{}", crate::ids::new_id().replace('-', ""))
    }

    /// Streamed sha256 + byte count of the SOURCE file (the verify reference).
    async fn source_sha256(path: &str) -> Result<(String, u64), AdapterError> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| AdapterError::Corruption(format!("source gone at verify: {e}")))?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| AdapterError::Corruption(format!("source read: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            total += n as u64;
        }
        Ok((hex_lower(&hasher.finalize()), total))
    }

    fn remote_path(endpoint: &str, source_path: &str) -> String {
        let dir = match endpoint {
            "root" | "" => "/apps/ium",
            other => other.trim_end_matches('/'),
        };
        let name = source_path.rsplit('/').next().unwrap_or(source_path);
        format!("{dir}/{name}")
    }

    /// The W5 host, refreshed at 90% of the 600 s assignment life.
    async fn upload_host(
        &self,
        endpoint: &str,
        session: &mut TeraboxSession,
    ) -> Result<String, AdapterError> {
        let fresh = match (&session.uhost, session.uhost_at) {
            (Some(h), Some(at)) if at.elapsed().as_secs() < 540 => h.clone(),
            _ => {
                let jt = self.js_token().await?;
                let h = self.locate_upload_host(&jt).await?;
                session.uhost = Some(h.clone());
                session.uhost_at = Some(Instant::now());
                self.insert_session(endpoint, session.clone());
                h
            }
        };
        Ok(fresh)
    }

    /// The streamed download (W8) — the verify route. Returns the bytes'
    /// sha256 hex + count. errno-2 bodies → [`AdapterError::Corruption`] via
    /// the caller's context.
    async fn download_stream(&self, path: &str) -> Result<(String, u64), AdapterError> {
        let jt = self.js_token().await?;
        let query = format!(
            "method=download&{}&path={}",
            self.api_query(&jt),
            uri_encode(path)
        );
        let resp = self
            .http
            .get(format!("{}/rest/2.0/pcs/file?{query}", self.cfg.base))
            .header("Cookie", self.cookie())
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("terabox download: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Self::map_status(status.as_u16(), "download"));
        }
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| AdapterError::Transient(format!("download stream: {e}")))?;
            // A JSON errno envelope in a 200 body is the wire's "file missing"
            // shape (errno 2 — W13/W8 rehearsal): fail Corruption, loud.
            if total == 0 && chunk.starts_with(b"{") {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&chunk) {
                    if v.get("errno").is_some() {
                        let errno = v.get("errno").and_then(|e| e.as_i64()).unwrap_or(-1);
                        let msg = v
                            .get("errmsg")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();
                        return Err(Self::map_errno(errno, &msg, "download"));
                    }
                }
            }
            hasher.update(&chunk);
            total += chunk.len() as u64;
        }
        Ok((hex_lower(&hasher.finalize()), total))
    }

    /// Drill cleanup (W7) — NOT an engine path: the drill deletes its own
    /// probe file so the owner's drive space stays clean.
    #[cfg(test)]
    async fn delete_file(&self, path: &str) -> Result<(), AdapterError> {
        let jt = self.js_token().await?;
        self.api_form(
            "/api/filemanager",
            &format!("opera=delete&async=1&{}", self.api_query(&jt)),
            &[("filelist", json!([path]).to_string())],
        )
        .await?;
        Ok(())
    }
}

fn hex_md5(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
impl TransferAdapter for TeraboxAdapter {
    fn name(&self) -> &'static str {
        "terabox"
    }

    fn capabilities(&self) -> Capability {
        // Resumable: the uploadid + journaled token re-derive the exact
        // done-set from the journal (D2), and re-PUTs are idempotent (W12).
        // NOT dynamic-chunks (the grid is prescribed, W10), NOT tail-race
        // (parts are atomic).
        Capability::new(Capability::RESUMABLE)
    }

    fn uniform_block(&self) -> Option<u64> {
        Some(TERABOX_BLOCK)
    }

    /// Open the session (W4): verify the on-disk size against the journal,
    /// self-serve the jsToken (W3), MD5-sweep the source, precreate with the
    /// per-slice block_list, pin the upload host (W5).
    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        let on_disk = tokio::fs::metadata(&spec.source_path)
            .await
            .map_err(|e| AdapterError::Corruption(format!("source unreadable: {e}")))?
            .len();
        if on_disk != spec.source_size {
            return Err(AdapterError::Corruption(format!(
                "source resized under the job: journal {} vs on-disk {on_disk}",
                spec.source_size
            )));
        }

        let path = Self::remote_path(&spec.endpoint, &spec.source_path);
        let dir = if spec.endpoint == "root" || spec.endpoint.is_empty() {
            "/apps/ium".to_string()
        } else {
            spec.endpoint.trim_end_matches('/').to_string()
        };

        // D2: one sweep feeds both the whole-file MD5 (unreferenced by the
        // wire today but cheap in the same pass) and the per-slice block_list.
        let (_whole, slices) = Self::md5_sweep(&spec.source_path).await?;
        let block_list = json!(slices).to_string();

        let jt = self.js_token().await?;
        let precreate = self
            .api_form(
                "/api/precreate",
                &self.api_query(&jt),
                &[
                    ("path", path.clone()),
                    ("autoinit", "1".to_string()),
                    ("target_path", format!("{dir}/")),
                    ("block_list", block_list),
                    ("size", spec.source_size.to_string()),
                ],
            )
            .await?;
        let uploadid = precreate
            .get("uploadid")
            .and_then(|u| u.as_str())
            .ok_or_else(|| {
                AdapterError::Unknown(
                    "precreate answered errno 0 without an uploadid (autoinit drift?)".into(),
                )
            })?
            .to_string();

        let uhost = self.locate_upload_host(&jt).await?;
        self.insert_session(
            &spec.endpoint,
            TeraboxSession {
                uploadid: uploadid.clone(),
                path,
                uhost: Some(uhost),
                uhost_at: Some(Instant::now()),
                fs_id: None,
            },
        );

        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(TERABOX_BLOCK),
            max_parallel: Some(1), // D1: serial lane; risk control exists
            min_chunk_bytes: Some(TERABOX_BLOCK),
            uniform_block: Some(TERABOX_BLOCK),
            session_token: Some(Self::session_token_json(
                &uploadid,
                &Self::remote_path(&spec.endpoint, &spec.source_path),
                None,
            )),
            concatenation: false,
        })
    }

    /// One part (W6): the whole chunk span rides as `partseq = lease.idx` on
    /// the assigned host; the echoed MD5 must match the local slice MD5.
    /// Parts are ATOMIC on this wire — the confirm credits the full span.
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let mut session = self.session(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(format!(
                "no terabox session for endpoint {} — reattach from the journal",
                spec.endpoint
            ))
        })?;
        let expected_slices = (spec.source_size.div_ceil(TERABOX_BLOCK) as usize).max(1);
        if lease.idx as usize >= expected_slices {
            return Err(AdapterError::Corruption(format!(
                "partseq {} beyond the {}-slice plan",
                lease.idx, expected_slices
            )));
        }
        let len = (lease.end - lease.begin) as usize;
        if len == 0 {
            // The 0-byte job (#5): a header-less lane — nothing to send.
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: 0,
                adapter_token: Some(session.uploadid.clone()),
            });
        }

        let mut file = tokio::fs::File::open(&spec.source_path)
            .await
            .map_err(|e| AdapterError::Corruption(format!("source unreadable: {e}")))?;
        file.seek(std::io::SeekFrom::Start(lease.begin))
            .await
            .map_err(|e| AdapterError::Corruption(format!("source seek: {e}")))?;
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes)
            .await
            .map_err(|e| AdapterError::Corruption(format!("source read: {e}")))?;
        progress.add(len as u64);

        let uhost = self.upload_host(&spec.endpoint, &mut session).await?;
        let boundary = Self::boundary();
        let file_name = session
            .path
            .rsplit('/')
            .next()
            .unwrap_or("part")
            .to_string();
        let body = Self::build_multipart(&bytes, &file_name, &boundary);
        let query = format!(
            "method=upload&app_id={}&channel=dubox&clienttype=0&web=1&path={}&uploadid={}&\
             uploadsign=0&partseq={}",
            self.cfg.app_id,
            uri_encode(&session.path),
            uri_encode(&session.uploadid),
            lease.idx
        );
        let resp = self
            .http
            .post(format!("https://{uhost}/rest/2.0/pcs/superfile2?{query}"))
            .header("Cookie", self.cookie())
            .header("User-Agent", USER_AGENT)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("superfile2: {e}")))?;
        let status = resp.status();
        let raw = resp
            .bytes()
            .await
            .map_err(|e| AdapterError::Transient(format!("superfile2 body: {e}")))?;
        if !status.is_success() {
            return Err(Self::map_status(status.as_u16(), "superfile2"));
        }
        let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| {
            AdapterError::Unknown(format!("superfile2: non-JSON envelope ({})", raw.len()))
        })?;
        if let Some(errno) = v.get("errno").and_then(|e| e.as_i64()) {
            if errno != 0 {
                return Err(Self::map_errno(errno, "superfile2 refused", "superfile2"));
            }
        }
        // W6: the echo IS the per-part integrity gate.
        let echoed = v
            .get("md5")
            .and_then(|m| m.as_str())
            .ok_or_else(|| AdapterError::Unknown("superfile2 without an md5 echo".into()))?;
        let local = hex_md5(&Md5::digest(&bytes));
        if !echoed.eq_ignore_ascii_case(&local) {
            return Err(AdapterError::Corruption(format!(
                "superfile2 md5 echo mismatch on partseq {}: echoed {echoed}, local {local}",
                lease.idx
            )));
        }

        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: len as u64,
            adapter_token: Some(session.uploadid.clone()),
        })
    }

    /// Verify (D3, W8/W13): download the stored file through the PCS route,
    /// hash it streamed, hold it against the source's sha256 AND size.
    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let session = self.session(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(
                "no terabox session at verify — the journaled token must be reattached".into(),
            )
        })?;
        let (got, total) = self.download_stream(&session.path).await?;
        let (want, source_len) = Self::source_sha256(&spec.source_path).await?;
        let ok = got == want && total == source_len && source_len == spec.source_size;
        let detail = Some(format!(
            "terabox: downloaded {total} bytes, sha256 {} (source {source_len} bytes, sha256 {want})",
            if got == want { "match" } else { "MISMATCH" }
        ));
        Ok(VerifyReport { ok, detail })
    }

    /// Restore the journaled session (D2) — never creates a session (the #12
    /// rule). The host is NOT restored (W5: ephemeral routing, expired at
    /// 600 s anyway) — the first part upload re-derives it via locateupload.
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        let Some((uploadid, path, fs_id)) = Self::parse_session_token(token) else {
            return Err(AdapterError::Unresumable(
                "journaled terabox token is not the JSON session shape".into(),
            ));
        };
        self.insert_session(
            endpoint,
            TeraboxSession {
                uploadid,
                path,
                uhost: None,
                uhost_at: None,
                fs_id,
            },
        );
        Ok(())
    }

    /// Create (W7) — the completeness gate, riding the manager's
    /// finalize-once machinery. Idempotent per session: a known fs_id is
    /// REUSED, never re-created. The block_list is recomputed from the source
    /// (deterministic over journal-immutable bytes, D2).
    async fn finalize(&self, spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        let session = self
            .session(&spec.endpoint)
            .ok_or_else(|| AdapterError::SessionLost("no terabox session at finalize".into()))?;
        if let Some(fs_id) = session.fs_id {
            return Ok(Some(fs_id));
        }
        let dir = session
            .path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_else(|| "/apps/ium".to_string());
        let (_, slices) = Self::md5_sweep(&spec.source_path).await?;
        let jt = self.js_token().await?;
        let v = self
            .api_form(
                "/api/create",
                &self.api_query(&jt),
                &[
                    ("path", session.path.clone()),
                    ("size", spec.source_size.to_string()),
                    ("uploadid", session.uploadid.clone()),
                    ("block_list", json!(slices).to_string()),
                    ("target_path", format!("{dir}/")),
                    ("local_mtime", now_secs().to_string()),
                    ("isdir", "0".to_string()),
                    ("rtype", "1".to_string()),
                ],
            )
            .await?;
        let fs_id = v
            .get("fs_id")
            .and_then(|f| f.as_u64())
            .ok_or_else(|| AdapterError::Unknown("create without fs_id".into()))?
            .to_string();
        self.insert_session(
            &spec.endpoint,
            TeraboxSession {
                fs_id: Some(fs_id.clone()),
                ..session
            },
        );
        Ok(Some(fs_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkPool;

    #[test]
    fn plan_is_uniform_tail() {
        let mib = 1024 * 1024;
        assert_eq!(
            ChunkPool::plan_uniform_tail(9 * mib, TERABOX_BLOCK),
            vec![(0, 4 * mib), (4 * mib, 8 * mib), (8 * mib, 9 * mib)]
        );
        assert_eq!(
            ChunkPool::plan_uniform_tail(8 * mib, TERABOX_BLOCK),
            vec![(0, 4 * mib), (4 * mib, 8 * mib)]
        );
        assert_eq!(ChunkPool::plan_uniform_tail(1, TERABOX_BLOCK), vec![(0, 1)]);
        // Issue #5 invariant: plan(0) → [(0, 0)].
        assert_eq!(ChunkPool::plan_uniform_tail(0, TERABOX_BLOCK), vec![(0, 0)]);
    }

    #[test]
    fn pool_new_uniform_matches_slice_count() {
        // The planned count equals the slice arithmetic ceil(size/4 MiB) for
        // every size — chunk i rides partseq i, 1:1 (W10).
        let mib = 1024 * 1024;
        for size in [0u64, 1, 4 * mib, 4 * mib + 1, 9 * mib, 100 * mib + 7] {
            let pool = ChunkPool::new_uniform(size, TERABOX_BLOCK);
            let want = (size.div_ceil(TERABOX_BLOCK) as usize).max(1);
            assert_eq!(pool.chunks().len(), want, "size {size}");
            let n = pool.chunks().len();
            for (i, c) in pool.chunks().iter().enumerate() {
                let len = c.end - c.begin;
                if i + 1 == n && size % TERABOX_BLOCK != 0 {
                    assert_eq!(len, size % TERABOX_BLOCK, "tail of size {size}");
                } else if n > 1 {
                    assert_eq!(len, TERABOX_BLOCK, "body chunk {i} of size {size}");
                }
            }
        }
    }

    #[test]
    fn session_token_roundtrip_and_final_injection() {
        let tok = TeraboxAdapter::session_token_json(
            "P1-MTAuNTcuMTE1LjM2OjE3OTM5MDkyNzM6MjUzMDQ2MDQyMDg1MTEzNjg3",
            "/apps/ium_drill/multipart9m.bin",
            None,
        );
        let (upid, path, fin) = TeraboxAdapter::parse_session_token(&tok).unwrap();
        assert_eq!(
            upid,
            "P1-MTAuNTcuMTE1LjM2OjE3OTM5MDkyNzM6MjUzMDQ2MDQyMDg1MTEzNjg3"
        );
        assert_eq!(path, "/apps/ium_drill/multipart9m.bin");
        assert_eq!(fin, None);
        let with_final = TeraboxAdapter::session_token_json(
            "P1-X",
            "/apps/ium_drill/multipart9m.bin",
            Some("254423615022544"),
        );
        let (_, _, fin) = TeraboxAdapter::parse_session_token(&with_final).unwrap();
        assert_eq!(fin.as_deref(), Some("254423615022544"));
        // A plain string is NOT this lane's shape (reattach refuses it).
        assert!(TeraboxAdapter::parse_session_token("plain-upload-id").is_none());
    }

    #[test]
    fn md5_vectors_from_python_hashlib() {
        let mib = 1024 * 1024;
        // echo -n "hello world" | md5sum
        assert_eq!(
            hex_md5(&Md5::digest(b"hello world")),
            "5eb63bbbe01eeed093cb22bb8f5acdc3"
        );
        // The 4096-byte pattern the Feishu adler32 vectors use.
        let pattern: Vec<u8> = (0..4096u32).map(|i| ((i * 7 + 3) % 256) as u8).collect();
        assert_eq!(
            hex_md5(&Md5::digest(&pattern)),
            "d105289f5617c241f52a191c6b2fc809"
        );
        // The sweep slices at exactly 4 MiB: 9 MiB → 3 MD5s (4+4+1); the
        // slice MD5s equal independent md5sums over the same ranges.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sweep.bin");
        let mut src = vec![0u8; 9 * mib];
        for (i, b) in src.iter_mut().enumerate() {
            *b = ((i as u64 * 13 + 5) % 256) as u8;
        }
        std::fs::write(&path, &src).unwrap();
        let (whole, slices) = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { TeraboxAdapter::md5_sweep(&path.to_string_lossy()).await })
            .expect("sweep");
        assert_eq!(slices.len(), 3);
        assert_eq!(
            whole,
            hex_md5(&Md5::digest(&src)),
            "the sweep's whole-file MD5 must equal a one-shot digest"
        );
        assert_eq!(
            slices[0],
            hex_md5(&Md5::digest(&src[0..4 * mib])),
            "slice 0 rides [0, 4 MiB)"
        );
        assert_eq!(
            slices[1],
            hex_md5(&Md5::digest(&src[4 * mib..8 * mib])),
            "slice 1 rides [4 MiB, 8 MiB)"
        );
        assert_eq!(
            slices[2],
            hex_md5(&Md5::digest(&src[8 * mib..])),
            "the tail slice carries the whole remainder"
        );
    }

    #[test]
    fn error_mapping_is_loud_and_exact() {
        let dead = TeraboxAdapter::map_errno(-6, "user not login", "precreate");
        assert!(matches!(dead, AdapterError::AuthRequired(_)));
        assert_eq!(dead.kind(), crate::retry::ErrorKind::AuthRequired);
        let risk = TeraboxAdapter::map_errno(4000023, "need verify", "precreate");
        assert!(matches!(risk, AdapterError::AuthRequired(_)));
        let missing = TeraboxAdapter::map_errno(2, "file does not exist", "download");
        assert!(matches!(missing, AdapterError::Corruption(_)));
        assert_eq!(missing.kind(), crate::retry::ErrorKind::Corruption);
        // errno 2 OUTSIDE the download context is NOT Corruption — loud
        // Unknown instead of a guessed classification.
        let elsewhere = TeraboxAdapter::map_errno(2, "no such file", "filemanager");
        assert!(matches!(elsewhere, AdapterError::Unknown(_)));
        let odd = TeraboxAdapter::map_errno(999999, "mystery", "precreate");
        assert!(matches!(odd, AdapterError::Unknown(_)));

        let throttled = TeraboxAdapter::map_status(429, "superfile2");
        assert!(matches!(throttled, AdapterError::Throttled { .. }));
        let transient = TeraboxAdapter::map_status(503, "superfile2");
        assert!(matches!(transient, AdapterError::Transient(_)));
        let auth = TeraboxAdapter::map_status(403, "download");
        assert!(matches!(auth, AdapterError::AuthRequired(_)));
    }

    #[test]
    fn js_token_extraction_serves_both_shapes() {
        // W3: URL-encoded (what the live /main carries) and raw.
        let enc = "html…fn%28%22A1B2C3D4A1B2C3D4A1B2C3D4A1B2C3D4%22%29…tail";
        assert_eq!(
            TeraboxAdapter::extract_js_token(enc),
            Some("A1B2C3D4A1B2C3D4A1B2C3D4A1B2C3D4".to_string())
        );
        let raw = "x</script>fn(\"0123456789abcdef0123456789abcdef\")y";
        assert_eq!(
            TeraboxAdapter::extract_js_token(raw),
            Some("0123456789abcdef0123456789abcdef".to_string())
        );
        // Short/garbage candidates are refused (the 128-hex real shape is
        // always >= 32 hex chars).
        assert_eq!(TeraboxAdapter::extract_js_token("fn(\"tooshort\")"), None);
        assert_eq!(TeraboxAdapter::extract_js_token("no token here"), None);
    }

    #[test]
    fn multipart_frame_shape() {
        let body = TeraboxAdapter::build_multipart(b"slice-bytes", "drill.bin", "BOUND");
        let s = String::from_utf8(body).unwrap();
        // The single binary part is named `file` with the remote file name,
        // framed by one boundary pair (W6: parameters ride the query).
        assert!(s.contains("name=\"file\"; filename=\"drill.bin\""));
        assert!(s.contains("Content-Type: application/octet-stream"));
        assert!(s.starts_with("--BOUND\r\n"));
        assert!(s.ends_with("slice-bytes\r\n--BOUND--\r\n"));
    }

    #[test]
    fn remote_path_semantics() {
        assert_eq!(
            TeraboxAdapter::remote_path("/apps/ium_drill", "/tmp/x/drill.bin"),
            "/apps/ium_drill/drill.bin"
        );
        assert_eq!(
            TeraboxAdapter::remote_path("/apps/ium_drill/", "/tmp/x/drill.bin"),
            "/apps/ium_drill/drill.bin",
            "a trailing slash on the endpoint never double-slashes"
        );
        assert_eq!(
            TeraboxAdapter::remote_path("root", "/tmp/x/drill.bin"),
            "/apps/ium/drill.bin",
            "'root' maps to the app space (precreate auto-creates target dirs — \
             proven live when /apps/ium_drill materialized on first use)"
        );
    }

    #[test]
    fn adapter_declares_honest_surface() {
        let cfg = TeraboxConfig::new("ndus".into(), "250528".into());
        let a = TeraboxAdapter::with_base(cfg, "http://127.0.0.1:1".into());
        assert_eq!(a.name(), "terabox");
        assert!(a.capabilities().has(Capability::RESUMABLE));
        assert!(!a.capabilities().has(Capability::DYNAMIC_CHUNKS));
        assert!(!a.capabilities().has(Capability::TAIL_RACE));
        // W10: the geometry is STATIC — declared before any probe exists.
        assert_eq!(a.uniform_block(), Some(TERABOX_BLOCK));
    }

    /// The LIVE drill (D5, env-gated): a real 9 MiB file through the real
    /// community wire — probe → out-of-order parts (W11) → journal-truth
    /// resume on a FRESH adapter instance (the crash-window shape) → create
    /// → download+sha256 verify byte-exact → cleanup (W7). Skips loud
    /// without `IUM_TERABOX_NDUS`.
    #[tokio::test]
    async fn terabox_live_drill() {
        let cfg = match TeraboxConfig::from_env() {
            Ok(c) => c,
            Err(_) => {
                eprintln!("terabox drill: IUM_TERABOX_NDUS not set — skip");
                return;
            }
        };
        let mib = 1024 * 1024;
        let size: u64 = 9 * mib; // → 3 slices (4+4+1)
        let dir = std::env::temp_dir().join(format!("ium-terabox-drill-{}", crate::ids::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ium-drill-9mib.bin");
        let mut src = vec![0u8; size as usize];
        for (i, b) in src.iter_mut().enumerate() {
            *b = ((i as u64 * 13 + 5) % 256) as u8;
        }
        std::fs::write(&path, &src).unwrap();

        let endpoint = "/apps/ium_drill";
        let spec = JobSpec {
            source_path: path.to_string_lossy().into(),
            source_size: size,
            adapter: "terabox".into(),
            endpoint: endpoint.into(),
        };
        let a = TeraboxAdapter::new(cfg.clone());

        // W4/W5: probe → prescribed geometry + journaled session token.
        let report = a.probe(&spec).await.expect("probe");
        assert_eq!(report.uniform_block, Some(TERABOX_BLOCK));
        assert_eq!(report.max_parallel, Some(1));
        let token = report.session_token.clone().expect("session token");
        let (upid, rpath, _) = TeraboxAdapter::parse_session_token(&token).unwrap();
        assert!(!upid.is_empty());
        assert_eq!(rpath, "/apps/ium_drill/ium-drill-9mib.bin");

        let plan = ChunkPool::plan_uniform_tail(size, TERABOX_BLOCK);
        assert_eq!(plan.len(), 3);
        let lease_of = |i: u32| Lease {
            idx: i,
            begin: plan[i as usize].0,
            end: plan[i as usize].1,
            resume_from: plan[i as usize].0,
            attempt: 0,
        };

        // W11: OUT-OF-ORDER — the tail slice (partseq 2) before 0 and 1;
        // W12 covers the crash-window re-send: partseq 0 goes TWICE.
        for i in [2u32, 0, 0, 1] {
            let progress = ProgressReporter::new();
            let confirm = a
                .upload_chunk(&spec, &lease_of(i), &progress)
                .await
                .unwrap_or_else(|e| panic!("part {i}: {e}"));
            assert_eq!(
                confirm.bytes_confirmed,
                plan[i as usize].1 - plan[i as usize].0
            );
            assert_eq!(confirm.idx, i);
            assert_eq!(progress.moved(), confirm.bytes_confirmed);
        }

        // Journal-truth resume (the crash-window shape): a FRESH adapter
        // (empty memory) reattaches from the JOURNALED token — no re-probe,
        // no re-upload of confirmed parts.
        let b = TeraboxAdapter::new(cfg.clone());
        b.reattach(endpoint, &token).await.expect("reattach");

        // D3: create → fs_id; download+sha256 byte-exact.
        let fs_id = b.finalize(&spec).await.expect("finalize").expect("fs id");
        assert!(!fs_id.is_empty());
        // Finalize-once idempotence: a second call reuses the fs_id.
        let again = b
            .finalize(&spec)
            .await
            .expect("finalize twice")
            .expect("fs id");
        assert_eq!(again, fs_id);
        let vr = b.verify(&spec).await.expect("verify");
        assert!(vr.ok, "verify must pass: {:?}", vr.detail);
        assert!(vr.detail.unwrap().contains("match"));

        // W7 cleanup: the drill deletes its own probe file.
        b.delete_file("/apps/ium_drill/ium-drill-9mib.bin")
            .await
            .expect("cleanup");
        std::fs::remove_dir_all(&dir).ok();
    }
}
