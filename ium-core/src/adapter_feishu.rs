//! The Feishu Drive lane (issue #56, M9 rung 1) — drive-v1 resumable
//! multipart upload as a first-class [`TransferAdapter`].
//!
//! # Wire facts (rehearsed LIVE 2026-11-05 against the real API, W0–W11)
//!
//! * **W1** `POST /drive/v1/files/upload_prepare` `{file_name,
//!   parent_type:"explorer", parent_node, size}` → `{upload_id, block_size,
//!   block_num}` — **block_size is platform-fixed at 4 MiB**, `block_num =
//!   ceil(size / 4 MiB)`; the transaction (and progress) lives **24 h**.
//! * **W2** `POST /drive/v1/files/upload_part` — **multipart/form-data**
//!   (not JSON, not query): `upload_id`, `seq` (0-based), `size` (actual
//!   part bytes), optional `checksum` (Adler-32, decimal string), binary
//!   part named **`file`**. Serial only (docs: no concurrency, 5 QPS).
//! * **W3** `POST /drive/v1/files/upload_finish` `{upload_id, block_num}` →
//!   `{file_token}`; missing blocks → 1062010 (finish IS the completeness
//!   gate — the engine's all-Confirmed boundary already guarantees it).
//! * **W4** No size is exposed by metas or folder lists — **verify =
//!   streamed download + sha256** (the strongest verify-before-declare).
//! * **W6** 9 MiB → `block_num=3` (4+4+1); the tail part uploads its actual
//!   size. **W7** out-of-order parts ACCEPTED (seq 1 before seq 0 → 200).
//! * **W11** even_split shapes are REJECTED: any non-tail part that is not
//!   exactly `block_size` fails 1062009 (probed: 3+3+3 refused). Hence the
//!   lane declares [`ProbeReport::uniform_block`] and the planner must run
//!   [`crate::chunk::ChunkPool::plan_uniform_tail`] — chunk *i* rides part `seq` *i*.
//!
//! # Design (the design note on #56, D1–D5)
//!
//! * **D2** session = the upload transaction: `upload_id` is the session
//!   token, journaled in the probe event as the JSON shape
//!   `{"upload_id":..,"block_num":..,"final":null}` — the manager's
//!   finalize-once machinery (the #50 crash-window repair) re-journals it
//!   with `final` = file_token, and `reattach` restores it verbatim
//!   (journal-truth, ADR-002; duplicates never re-sent).
//! * **D4** error mapping is loud: 1061021 (24 h expiry) →
//!   [`AdapterError::ExpiredSession`] (the taxonomy's exact case), 1061045 →
//!   [`AdapterError::Busy`] (the #44 wait-not-failure pattern), 1062009 →
//!   [`AdapterError::Corruption`], 1061073/1061004 →
//!   [`AdapterError::AuthRequired`].
//! * **D5** credentials come from env (`IUM_FEISHU_APP_ID` /
//!   `IUM_FEISHU_APP_SECRET`); the live drill is env-gated and skips loud
//!   without them (the S3 kill-9 pattern) — no secrets ever in the repo.

use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::JobSpec;

/// The platform-fixed Feishu block size (W1: 4194304 observed on 4 KiB,
/// 8 KiB, 9 MiB payloads; docs: "平台固定以 4MB 的大小对文件进行分片").
pub const FEISHU_BLOCK: u64 = 4 * 1024 * 1024;

const DEFAULT_BASE: &str = "https://open.feishu.cn/open-apis";

/// Feishu app credentials (the tenant-credential identity — no user OAuth
/// for uploads into the app's own drive space, W0).
#[derive(Debug, Clone)]
pub struct FeishuConfig {
    /// The app id (`cli_…` — the open-platform application identifier).
    pub app_id: String,
    /// The app secret (never journaled, never logged — env-injected, D5).
    pub app_secret: String,
}

impl FeishuConfig {
    /// Build from explicit credentials (tests, embedders).
    pub fn new(app_id: String, app_secret: String) -> Self {
        Self { app_id, app_secret }
    }

    /// Read from the environment (`IUM_FEISHU_APP_ID` /
    /// `IUM_FEISHU_APP_SECRET`) — the env-gated drill lane (D5).
    pub fn from_env() -> Result<Self, AdapterError> {
        let app_id = std::env::var("IUM_FEISHU_APP_ID")
            .map_err(|_| AdapterError::AuthRequired("IUM_FEISHU_APP_ID not set".into()))?;
        let app_secret = std::env::var("IUM_FEISHU_APP_SECRET")
            .map_err(|_| AdapterError::AuthRequired("IUM_FEISHU_APP_SECRET not set".into()))?;
        Ok(Self { app_id, app_secret })
    }
}

/// A cached `tenant_access_token` (2 h Feishu expiry; refreshed at 90% life
/// so an in-flight 4 MiB part never crosses the cliff).
#[derive(Debug, Clone)]
pub struct TokenCache {
    /// The bearer value (`t-…`) sent as `Authorization: Bearer …`.
    pub value: String,
    /// The server-declared lifetime in seconds (observed: 7200).
    pub expire_secs: u64,
    /// When the token was fetched (the 90%-life clock origin).
    pub fetched_at: Instant,
}

impl TokenCache {
    /// A cache entry stamped at fetch time.
    pub fn new(value: String, expire_secs: u64, fetched_at: Instant) -> Self {
        Self {
            value,
            expire_secs,
            fetched_at,
        }
    }

    /// Refresh needed once 90% of the declared life is spent.
    pub fn needs_refresh(&self) -> bool {
        self.fetched_at.elapsed().as_secs() >= self.expire_secs * 9 / 10
    }
}

/// One upload transaction per endpoint (the in-memory mirror of the
/// journaled probe token — restored by `reattach`, never re-probed).
#[derive(Debug, Clone)]
struct FeishuSession {
    upload_id: String,
    block_num: u32,
    file_token: Option<String>,
}

/// The adapter. One `Mutex`-guarded session map keyed by the job endpoint
/// (the tus pattern): probe inserts, `reattach` restores from the journal.
pub struct FeishuAdapter {
    http: reqwest::Client,
    base: String,
    cfg: FeishuConfig,
    token: tokio::sync::Mutex<Option<TokenCache>>,
    sessions: Mutex<HashMap<String, FeishuSession>>,
}

impl FeishuAdapter {
    /// Production constructor against the real Feishu open platform.
    pub fn new(cfg: FeishuConfig) -> Self {
        Self::with_base(cfg, DEFAULT_BASE.to_string())
    }

    /// Constructor with an explicit base URL (loopback tests).
    pub fn with_base(cfg: FeishuConfig, base: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client builds"),
            base,
            cfg,
            token: tokio::sync::Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The journaled session token (D2): JSON so the manager's
    /// finalize-once machinery recognizes the shape and its crash-window
    /// repair can inject `final` = file_token before reattach.
    pub fn session_token_json(upload_id: &str, block_num: u32, file_token: Option<&str>) -> String {
        json!({
            "upload_id": upload_id,
            "block_num": block_num,
            "final": file_token,
        })
        .to_string()
    }

    /// Inverse of [`FeishuAdapter::session_token_json`]; `None` for any
    /// non-JSON token (a plain string is not this lane's shape).
    pub fn parse_session_token(token: &str) -> Option<(String, u32, Option<String>)> {
        let v: serde_json::Value = serde_json::from_str(token).ok()?;
        let upload_id = v.get("upload_id")?.as_str()?.to_string();
        let block_num = v.get("block_num")?.as_u64()? as u32;
        let file_token = v.get("final").and_then(|f| f.as_str()).map(String::from);
        Some((upload_id, block_num, file_token))
    }

    /// Adler-32 over a part (W2's optional-but-cheap integrity field; the
    /// wire sends the decimal string). Pinned against python3 zlib in tests.
    pub fn adler32(data: &[u8]) -> u32 {
        const MOD: u32 = 65521;
        let mut s1: u32 = 1;
        let mut s2: u32 = 0;
        for &b in data {
            s1 = (s1 + b as u32) % MOD;
            s2 = (s2 + s1) % MOD;
        }
        (s2 << 16) | s1
    }

    /// Wire-code → retry-taxonomy mapping (D4). Loud, never guessed:
    /// 1061045 → Busy (wait, uncharged), 1061021 → ExpiredSession (the 24 h
    /// cliff — the taxonomy's exact case), 1062008/1062009 → Corruption
    /// (facts changed — never retried), 1061073/1061004 → AuthRequired,
    /// internal/transient codes → Transient, sequencing bugs → Unknown.
    pub fn map_feishu_error(code: i64, msg: &str) -> AdapterError {
        match code {
            1061045 => AdapterError::Busy(format!("feishu 1061045 can-retry: {msg}")),
            1061021 => {
                AdapterError::ExpiredSession(format!("feishu 1061021 upload id expired: {msg}"))
            }
            1062008 | 1062009 => {
                AdapterError::Corruption(format!("feishu {code} checksum/size mismatch: {msg}"))
            }
            1061073 | 1061004 => {
                AdapterError::AuthRequired(format!("feishu {code} no scope/permission: {msg}"))
            }
            1061001 | 1061006 | 1064230 => {
                AdapterError::Transient(format!("feishu {code} internal/transient: {msg}"))
            }
            _ => AdapterError::Unknown(format!("feishu {code}: {msg}")),
        }
    }

    /// The multipart body for `upload_part` (W2): form fields in wire order,
    /// binary part named **`file`** — the wire answered 1061002 to any other
    /// binary field name (probed live).
    pub(crate) fn build_multipart(
        fields: &[(&str, String)],
        file_bytes: &[u8],
        boundary: &str,
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(file_bytes.len() + 512);
        for (name, value) in fields {
            out.extend_from_slice(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                    .as_bytes(),
            );
        }
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"block\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        out.extend_from_slice(file_bytes);
        out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        out
    }

    /// A fresh boundary (uuid-shaped; uniqueness is all the wire needs).
    fn boundary() -> String {
        let id = crate::ids::new_id();
        format!("ium{}", id.replace('-', ""))
    }

    /// The cached tenant token (fetch + 90%-life refresh, D-token).
    async fn tenant_token(&self) -> Result<String, AdapterError> {
        {
            let guard = self.token.lock().await;
            if let Some(c) = guard.as_ref() {
                if !c.needs_refresh() {
                    return Ok(c.value.clone());
                }
            }
        }
        let body = json!({
            "app_id": self.cfg.app_id,
            "app_secret": self.cfg.app_secret,
        });
        let resp = self
            .http
            .post(format!(
                "{}/auth/v3/tenant_access_token/internal",
                self.base
            ))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("token endpoint: {e}")))?;
        let raw = resp
            .bytes()
            .await
            .map_err(|e| AdapterError::Transient(format!("token body: {e}")))?;
        let v: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|e| AdapterError::Transient(format!("token body: {e}")))?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(AdapterError::AuthRequired(format!(
                "feishu token rejected (code {code}): {}",
                v.get("msg").and_then(|m| m.as_str()).unwrap_or("?")
            )));
        }
        let value = v
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                AdapterError::Unknown("token response without tenant_access_token".into())
            })?
            .to_string();
        let expire = v.get("expire").and_then(|e| e.as_u64()).unwrap_or(7200);
        *self.token.lock().await = Some(TokenCache::new(value.clone(), expire, Instant::now()));
        Ok(value)
    }

    /// One authenticated JSON call → the parsed envelope (status always 200
    /// on Feishu; the `code` field is the real verdict).
    async fn call_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, AdapterError> {
        let token = self.tenant_token().await?;
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base, path))
            .header("Authorization", format!("Bearer {token}"));
        if let Some(b) = body {
            req = req
                .header("Content-Type", "application/json")
                .body(b.to_string());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("feishu call {path}: {e}")))?;
        let raw = resp
            .bytes()
            .await
            .map_err(|e| AdapterError::Transient(format!("feishu body {path}: {e}")))?;
        let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| {
            AdapterError::Unknown(format!(
                "feishu {path}: non-JSON envelope ({} bytes)",
                raw.len()
            ))
        })?;
        Ok(v)
    }

    fn session(&self, endpoint: &str) -> Option<FeishuSession> {
        self.sessions.lock().unwrap().get(endpoint).cloned()
    }

    fn insert_session(&self, endpoint: &str, s: FeishuSession) {
        self.sessions
            .lock()
            .unwrap()
            .insert(endpoint.to_string(), s);
    }

    /// Streamed sha256 of the SOURCE file (the verify reference).
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
        Ok((hex(&hasher.finalize()), total))
    }

    /// Drill cleanup (W5) — NOT an engine path: the drill deletes its own
    /// probe file so the owner's drive space stays clean.
    #[cfg(test)]
    async fn delete_file(&self, file_token: &str) -> Result<(), AdapterError> {
        let v = self
            .call_json(
                reqwest::Method::DELETE,
                &format!("/drive/v1/files/{file_token}?type=file"),
                None,
            )
            .await?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(Self::map_feishu_error(
                code,
                v.get("msg").and_then(|m| m.as_str()).unwrap_or("?"),
            ));
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[async_trait]
impl TransferAdapter for FeishuAdapter {
    fn name(&self) -> &'static str {
        "feishu_drive"
    }

    fn capabilities(&self) -> Capability {
        // Resumable: the 24 h transaction + journaled upload_id re-derive
        // the exact done-set from the journal (D2). NOT dynamic-chunks (the
        // grid is prescribed, W11), NOT tail-race (parts are atomic; a
        // racing re-send would double a part for zero feedback).
        Capability::new(Capability::RESUMABLE)
    }

    fn uniform_block(&self) -> Option<u64> {
        Some(FEISHU_BLOCK)
    }

    /// Prepare (W1): resolve the parent (the app's own root when the
    /// endpoint is "root"), verify the on-disk size against the journal,
    /// open the transaction, pin the geometry the wire returns.
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

        let parent = if spec.endpoint == "root" {
            let v = self
                .call_json(
                    reqwest::Method::GET,
                    "/drive/explorer/v2/root_folder/meta",
                    None,
                )
                .await?;
            let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            if code != 0 {
                return Err(Self::map_feishu_error(
                    code,
                    v.get("msg").and_then(|m| m.as_str()).unwrap_or("?"),
                ));
            }
            v.pointer("/data/token")
                .and_then(|t| t.as_str())
                .ok_or_else(|| AdapterError::Unknown("root meta without token".into()))?
                .to_string()
        } else {
            spec.endpoint.clone()
        };

        let body = json!({
            "file_name": file_name_of(&spec.source_path),
            "parent_type": "explorer",
            "parent_node": parent,
            "size": spec.source_size,
        });
        let v = self
            .call_json(
                reqwest::Method::POST,
                "/drive/v1/files/upload_prepare",
                Some(body),
            )
            .await?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(Self::map_feishu_error(
                code,
                v.get("msg").and_then(|m| m.as_str()).unwrap_or("?"),
            ));
        }
        let upload_id = v
            .pointer("/data/upload_id")
            .and_then(|u| u.as_str())
            .ok_or_else(|| AdapterError::Unknown("prepare without upload_id".into()))?
            .to_string();
        let block_size = v
            .pointer("/data/block_size")
            .and_then(|b| b.as_u64())
            .ok_or_else(|| AdapterError::Unknown("prepare without block_size".into()))?;
        let block_num = v
            .pointer("/data/block_num")
            .and_then(|b| b.as_u64())
            .ok_or_else(|| AdapterError::Unknown("prepare without block_num".into()))?
            as u32;

        // The wire's arithmetic must agree with the prescribed geometry —
        // any drift means the platform changed under us (loud, never retried).
        if block_size != FEISHU_BLOCK || block_num as u64 != spec.source_size.div_ceil(FEISHU_BLOCK)
        {
            return Err(AdapterError::Corruption(format!(
                "prepare geometry drifted: block_size {block_size}, block_num {block_num}, \
                 expected block_size {FEISHU_BLOCK}, block_num {}",
                spec.source_size.div_ceil(FEISHU_BLOCK)
            )));
        }

        self.insert_session(
            &spec.endpoint,
            FeishuSession {
                upload_id: upload_id.clone(),
                block_num,
                file_token: None,
            },
        );

        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(FEISHU_BLOCK),
            max_parallel: Some(1), // W2: serial parts, 5 QPS, no concurrency
            min_chunk_bytes: Some(FEISHU_BLOCK),
            uniform_block: Some(FEISHU_BLOCK),
            session_token: Some(Self::session_token_json(&upload_id, block_num, None)),
            concatenation: false,
        })
    }

    /// One part (W2): the whole chunk span rides as `seq = lease.idx` with
    /// its Adler-32. Parts are ATOMIC on this wire — in-chunk partial
    /// resumes never exist, so the confirm credits the full span exactly
    /// once the server took the part (200/code 0).
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let session = self.session(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(format!(
                "no feishu session for endpoint {} — reattach from the journal",
                spec.endpoint
            ))
        })?;
        if lease.idx as u64 >= session.block_num as u64 {
            return Err(AdapterError::Corruption(format!(
                "part seq {} beyond block_num {}",
                lease.idx, session.block_num
            )));
        }
        let len = (lease.end - lease.begin) as usize;
        if len == 0 {
            // The 0-byte job (#5): a header-less lane — nothing to send.
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: 0,
                adapter_token: Some(session.upload_id.clone()),
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

        let boundary = Self::boundary();
        let checksum = Self::adler32(&bytes).to_string();
        let body = Self::build_multipart(
            &[
                ("upload_id", session.upload_id.clone()),
                ("seq", lease.idx.to_string()),
                ("size", len.to_string()),
                ("checksum", checksum),
            ],
            &bytes,
            &boundary,
        );

        let token = self.tenant_token().await?;
        let resp = self
            .http
            .post(format!("{}/drive/v1/files/upload_part", self.base))
            .header("Authorization", format!("Bearer {token}"))
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("part upload: {e}")))?;
        let raw = resp
            .bytes()
            .await
            .map_err(|e| AdapterError::Transient(format!("part body: {e}")))?;
        let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| {
            AdapterError::Unknown(format!("part: non-JSON envelope ({})", raw.len()))
        })?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(Self::map_feishu_error(
                code,
                v.get("msg").and_then(|m| m.as_str()).unwrap_or("?"),
            ));
        }

        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: len as u64,
            adapter_token: Some(session.upload_id.clone()),
        })
    }

    /// Verify (D3, W4/W10): no size/ETag exists on this wire — download the
    /// stored file, hash it streamed, and hold it against the source's
    /// sha256. The strongest form of verify-before-declare.
    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let session = self.session(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(
                "no feishu session at verify — the journaled token must be reattached".into(),
            )
        })?;
        let file_token = session.file_token.ok_or_else(|| {
            AdapterError::Unresumable(
                "verify before finalize — no file_token on the session".into(),
            )
        })?;

        let token = self.tenant_token().await?;
        let resp = self
            .http
            .get(format!(
                "{}/drive/v1/files/{file_token}/download",
                self.base
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("verify download: {e}")))?;
        if !resp.status().is_success() {
            return Err(AdapterError::Transient(format!(
                "verify download: HTTP {}",
                resp.status()
            )));
        }

        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| AdapterError::Transient(format!("verify stream: {e}")))?;
            hasher.update(&chunk);
            total += chunk.len() as u64;
        }
        let got = hex(&hasher.finalize());

        let (want, source_len) = Self::source_sha256(&spec.source_path).await?;
        let ok = got == want && total == source_len && source_len == spec.source_size;
        let detail = Some(format!(
            "feishu: downloaded {total} bytes, sha256 {} (source {source_len} bytes, sha256 {want})",
            if got == want { "match" } else { "MISMATCH" }
        ));
        Ok(VerifyReport { ok, detail })
    }

    /// Restore the journaled transaction (D2) — never creates a session
    /// (the #12 rule): the token is the JSON shape this lane journals.
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        let Some((upload_id, block_num, file_token)) = Self::parse_session_token(token) else {
            return Err(AdapterError::Unresumable(
                "journaled feishu token is not the JSON session shape".into(),
            ));
        };
        self.insert_session(
            endpoint,
            FeishuSession {
                upload_id,
                block_num,
                file_token,
            },
        );
        Ok(())
    }

    /// Finish (W3) — the completeness gate, riding the manager's
    /// finalize-once machinery (the #50 pattern: journaled intent/done,
    /// crash-window repair re-journals the probe token with `final`).
    /// Idempotent per session: a known file_token is REUSED, never re-finished.
    async fn finalize(&self, spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        let session = self
            .session(&spec.endpoint)
            .ok_or_else(|| AdapterError::SessionLost("no feishu session at finalize".into()))?;
        if let Some(ft) = session.file_token {
            return Ok(Some(ft));
        }
        let v = self
            .call_json(
                reqwest::Method::POST,
                "/drive/v1/files/upload_finish",
                Some(json!({
                    "upload_id": session.upload_id,
                    "block_num": session.block_num,
                })),
            )
            .await?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(Self::map_feishu_error(
                code,
                v.get("msg").and_then(|m| m.as_str()).unwrap_or("?"),
            ));
        }
        let file_token = v
            .pointer("/data/file_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| AdapterError::Unknown("finish without file_token".into()))?
            .to_string();
        self.insert_session(
            &spec.endpoint,
            FeishuSession {
                upload_id: session.upload_id,
                block_num: session.block_num,
                file_token: Some(file_token.clone()),
            },
        );
        Ok(Some(file_token))
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
            ChunkPool::plan_uniform_tail(9 * mib, 4 * mib),
            vec![(0, 4 * mib), (4 * mib, 8 * mib), (8 * mib, 9 * mib)]
        );
        assert_eq!(
            ChunkPool::plan_uniform_tail(8 * mib, 4 * mib),
            vec![(0, 4 * mib), (4 * mib, 8 * mib)]
        );
        assert_eq!(ChunkPool::plan_uniform_tail(1, 4 * mib), vec![(0, 1)]);
        // Issue #5 invariant: plan(0) → [(0, 0)].
        assert_eq!(ChunkPool::plan_uniform_tail(0, 4 * mib), vec![(0, 0)]);
    }

    #[test]
    fn pool_new_uniform_matches_prepare_block_num() {
        // The planned count equals the prepare arithmetic ceil(size/4 MiB)
        // for every size — chunk i rides part seq i, 1:1 (W1/W6).
        let mib = 1024 * 1024;
        for size in [0u64, 1, 4 * mib, 4 * mib + 1, 9 * mib, 100 * mib + 7] {
            let pool = ChunkPool::new_uniform(size, FEISHU_BLOCK);
            let want = (size.div_ceil(FEISHU_BLOCK) as usize).max(1);
            assert_eq!(pool.chunks().len(), want, "size {size}");
            // The tail chunk carries the whole remainder; non-tail are exact.
            let n = pool.chunks().len();
            for (i, c) in pool.chunks().iter().enumerate() {
                let len = c.end - c.begin;
                if i + 1 == n && size % FEISHU_BLOCK != 0 {
                    assert_eq!(len, size % FEISHU_BLOCK, "tail of size {size}");
                } else if n > 1 {
                    assert_eq!(len, FEISHU_BLOCK, "body chunk {i} of size {size}");
                }
            }
        }
    }

    #[test]
    fn probe_report_declares_uniform_block() {
        let r = ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(FEISHU_BLOCK),
            max_parallel: Some(1),
            min_chunk_bytes: Some(FEISHU_BLOCK),
            uniform_block: Some(FEISHU_BLOCK),
            session_token: None,
            concatenation: false,
        };
        assert_eq!(r.uniform_block, Some(FEISHU_BLOCK));
    }

    #[test]
    fn session_token_roundtrip_and_final_injection() {
        let tok = FeishuAdapter::session_token_json("7111211691345512356", 3, None);
        let (uid, n, fin) = FeishuAdapter::parse_session_token(&tok).unwrap();
        assert_eq!(uid, "7111211691345512356");
        assert_eq!(n, 3);
        assert_eq!(fin, None);
        let with_final =
            FeishuAdapter::session_token_json("7111211691345512356", 3, Some("boxcnFILE"));
        let (_, _, fin) = FeishuAdapter::parse_session_token(&with_final).unwrap();
        assert_eq!(fin.as_deref(), Some("boxcnFILE"));
        // A plain string is NOT this lane's shape (reattach refuses it).
        assert!(FeishuAdapter::parse_session_token("plain-upload-id").is_none());
    }

    #[test]
    fn adler32_vectors_from_python_zlib() {
        assert_eq!(FeishuAdapter::adler32(b"hello world"), 436929629);
        let pattern: Vec<u8> = (0..4096u32).map(|i| ((i * 7 + 3) % 256) as u8).collect();
        assert_eq!(FeishuAdapter::adler32(&pattern), 2585131114);
    }

    #[test]
    fn error_mapping_is_loud_and_exact() {
        let busy = FeishuAdapter::map_feishu_error(1061045, "can retry");
        assert!(matches!(busy, AdapterError::Busy(_)));
        assert_eq!(busy.kind(), crate::retry::ErrorKind::Busy);
        let expired = FeishuAdapter::map_feishu_error(1061021, "upload id expire");
        assert!(matches!(expired, AdapterError::ExpiredSession(_)));
        assert_eq!(expired.kind(), crate::retry::ErrorKind::ExpiredSession);
        let size = FeishuAdapter::map_feishu_error(1062009, "size inconsistent");
        assert!(matches!(size, AdapterError::Corruption(_)));
        let auth = FeishuAdapter::map_feishu_error(1061073, "no scope auth");
        assert!(matches!(auth, AdapterError::AuthRequired(_)));
        let internal = FeishuAdapter::map_feishu_error(1061001, "internal error");
        assert!(matches!(internal, AdapterError::Transient(_)));
        let odd = FeishuAdapter::map_feishu_error(999999, "mystery");
        assert!(matches!(odd, AdapterError::Unknown(_)));
    }

    #[test]
    fn token_cache_refreshes_at_ninety_percent_life() {
        let fresh = TokenCache::new("t".into(), 7200, Instant::now());
        assert!(!fresh.needs_refresh());
        let old = TokenCache::new("t".into(), 7200, Instant::now() - Duration::from_secs(6481));
        assert!(old.needs_refresh());
        let almost = TokenCache::new("t".into(), 7200, Instant::now() - Duration::from_secs(6479));
        assert!(!almost.needs_refresh());
    }

    #[test]
    fn multipart_frame_shape() {
        let body = FeishuAdapter::build_multipart(
            &[
                ("upload_id", "U123".into()),
                ("seq", "0".into()),
                ("size", "5".into()),
                ("checksum", "12345".into()),
            ],
            b"bytes",
            "BOUND",
        );
        let s = String::from_utf8(body).unwrap();
        // Field order (wire order), the binary part named `file`, one boundary.
        let uid = s.find("name=\"upload_id\"").unwrap();
        let seq = s.find("name=\"seq\"").unwrap();
        let size = s.find("name=\"size\"").unwrap();
        let cks = s.find("name=\"checksum\"").unwrap();
        let file = s.find("name=\"file\"").unwrap();
        assert!(uid < seq && seq < size && size < cks && cks < file);
        assert!(s.contains("--BOUND--\r\n"));
        assert!(s.ends_with("bytes\r\n--BOUND--\r\n"));
    }

    #[test]
    fn adapter_declares_honest_surface() {
        let cfg = FeishuConfig::new("app".into(), "secret".into());
        let a = FeishuAdapter::with_base(cfg, "http://127.0.0.1:1".into());
        assert_eq!(a.name(), "feishu_drive");
        assert!(a.capabilities().has(Capability::RESUMABLE));
        assert!(!a.capabilities().has(Capability::DYNAMIC_CHUNKS));
        assert!(!a.capabilities().has(Capability::TAIL_RACE));
        // W1: the geometry is STATIC — declared before any probe exists
        // (create_job consults it directly).
        assert_eq!(a.uniform_block(), Some(FEISHU_BLOCK));
    }

    /// The LIVE drill (D5, env-gated): a real 9 MiB file through the real
    /// drive-v1 wire — probe → out-of-order parts (W7) → journal-truth
    /// resume on a FRESH adapter instance (the crash-window shape) →
    /// finish → download+sha256 verify byte-exact → cleanup (W5).
    /// Skips loud without `IUM_FEISHU_APP_ID`/`IUM_FEISHU_APP_SECRET`
    /// (the S3 kill-9 drill pattern; runs on the drill lane with vault
    /// credentials).
    #[tokio::test]
    async fn feishu_drive_live_drill() {
        let cfg = match FeishuConfig::from_env() {
            Ok(c) => c,
            Err(_) => {
                eprintln!("feishu drill: IUM_FEISHU_APP_ID/SECRET not set — skip");
                return;
            }
        };
        let mib = 1024 * 1024;
        let size: u64 = 9 * mib; // → block_num 3 (4+4+1)
        let dir = std::env::temp_dir().join(format!("ium-feishu-drill-{}", crate::ids::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ium-drill-9mib.bin");
        let mut src = vec![0u8; size as usize];
        for (i, b) in src.iter_mut().enumerate() {
            *b = ((i as u64 * 13 + 5) % 256) as u8;
        }
        std::fs::write(&path, &src).unwrap();

        let endpoint = "root";
        let spec = JobSpec {
            source_path: path.to_string_lossy().into(),
            source_size: size,
            adapter: "feishu_drive".into(),
            endpoint: endpoint.into(),
        };
        let a = FeishuAdapter::new(cfg.clone());

        // W1: probe → prescribed geometry + journaled session token.
        let report = a.probe(&spec).await.expect("probe");
        assert_eq!(report.uniform_block, Some(FEISHU_BLOCK));
        assert_eq!(report.max_parallel, Some(1));
        let token = report.session_token.clone().expect("session token");
        let (uid, block_num, _) = FeishuAdapter::parse_session_token(&token).unwrap();
        assert_eq!(block_num, 3);
        assert!(!uid.is_empty());

        let plan = ChunkPool::plan_uniform_tail(size, FEISHU_BLOCK);
        assert_eq!(plan.len(), 3);
        let lease_of = |i: u32| Lease {
            idx: i,
            begin: plan[i as usize].0,
            end: plan[i as usize].1,
            resume_from: plan[i as usize].0,
            attempt: 0,
        };

        // W7: OUT-OF-ORDER — part 2 (the 1 MiB tail) before parts 0 and 1.
        for i in [2u32, 0, 1] {
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
        }

        // Journal-truth resume (the crash-window shape): a FRESH adapter
        // (empty memory) reattaches from the JOURNALED token — no re-probe,
        // no re-upload of confirmed parts (all 3 are done here, so finish
        // must succeed on B's memory alone).
        let b = FeishuAdapter::new(cfg.clone());
        b.reattach(endpoint, &token).await.expect("reattach");

        // D3/W10: finish → file_token; download+sha256 byte-exact.
        let file_token = b
            .finalize(&spec)
            .await
            .expect("finalize")
            .expect("file token");
        assert!(!file_token.is_empty());
        let vr = b.verify(&spec).await.expect("verify");
        assert!(vr.ok, "verify must pass: {:?}", vr.detail);
        assert!(vr.detail.unwrap().contains("match"));

        // W5 cleanup: the drill deletes its own probe file.
        b.delete_file(&file_token).await.expect("cleanup");
        std::fs::remove_dir_all(&dir).ok();
    }
}
