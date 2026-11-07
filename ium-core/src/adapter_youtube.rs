//! YouTube lane (#85) — IUM's first NATIVE OAuth adapter: the YouTube
//! Data API v3 resumable upload protocol, ridden directly (rclone has no
//! youtube backend, so the browser sign-in cannot borrow it — the sign-in
//! is `google.rs`'s own loopback flow).
//!
//! The wire (Google's own reliability story, mirrored honestly):
//! * **init** — `POST …/videos?uploadType=resumable&part=snippet,status`
//!   with the metadata JSON → `200` + a `Location` session URI (journaled
//!   as the probe's `session_token` — ADR-002: the journal is the truth);
//! * **chunks** — sequential `PUT`s of 256 KiB-multiple ranges with
//!   `Content-Range: bytes a-b/total`; `308 Resume Incomplete` + a `Range`
//!   header is the server's own acknowledgment of the prefix it holds;
//! * **resume** — after any death: `PUT` with `Content-Range: bytes
//!   */total` asks what the server already holds, the `Range` answer is
//!   journaled through the normal chunk-confirm composition, and the next
//!   `PUT` starts where the server is — never a silent restart from zero;
//! * **verify-before-declare** — `videos.list?part=status&id=…` must show
//!   the resource (`uploaded`/`processed`). YouTube offers NO server-side
//!   file checksum, so the lane says exactly what it verified: existence +
//!   status, honest-by-capability — never a fake byte-match claim.
//!
//! One session is ONE sequential byte stream: `max_parallel = Some(1)`, and
//! a chunk whose begin lies ahead of the server's received prefix is
//! `ExpiredSession` (the session's truth outranks the plan's optimism).
//!
//! Credentials: `IUM_GOOGLE_CLIENT_ID` + `IUM_GOOGLE_CLIENT_SECRET` +
//! `IUM_YOUTUBE_TOKEN` (the [`crate::google::TokenSet`] JSON — refresh survives
//! restarts, the access token refreshes lazily on expiry). HONEST LIMIT:
//! until IUM ships a registered (Google-verified) client, the client id and
//! secret are a ONE-TIME sheet setup; after that, connect is one click.
//! `youtube.upload` is a sensitive scope — unverified projects serve test
//! users only (the sheet says so in one sentence).

use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::{Client, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::google::{GoogleClient, TokenSet};
use crate::JobSpec;

/// The registry key — the create-job form's select shows this once the
/// lane is connected.
pub const LANE: &str = "youtube";

/// The chunk size this lane plans at: 8 MiB — a comfortable multiple of
/// the protocol's 256 KiB grain.
pub const CHUNK: u64 = 8 * 1024 * 1024;

/// The protocol's smallest legal chunk (256 KiB) — the lane floor the
/// planner obeys (ADR-011: adapter declares, planner obeys).
pub const MIN_CHUNK: u64 = 262_144;

/// One session is ONE sequential byte stream — never two writers.
pub const MAX_PARALLEL: u32 = 1;

/// The metadata default: PRIVATE. A video the world never sees by accident.
pub const PRIVACY_DEFAULT: &str = "private";

/// The category default (22 = People & Blogs) — a harmless, overridable
/// constant; the lane never guesses beyond it.
pub const CATEGORY_DEFAULT: &str = "22";

/// The resumable metadata body: `part=snippet,status` — title from the
/// source filename (sanitized), privacy defaulting to PRIVATE.
pub fn metadata_body(title: &str, privacy: &str, category: &str) -> String {
    format!(
        "{{\"snippet\":{{\"title\":{},\"categoryId\":\"{}\"}},\"status\":{{\"privacyStatus\":\"{}\",\"selfDeclaredMadeForKids\":false}}}}",
        serde_json::to_string(title).unwrap_or_else(|_| "\"Upload\"".into()),
        category,
        privacy,
    )
}

/// The `Content-Range` of one chunk PUT (end INCLUSIVE, Google's spelling).
pub fn content_range(start: u64, end_inclusive: u64, total: u64) -> String {
    format!("bytes {start}-{end_inclusive}/{total}")
}

/// The resume probe's `Content-Range` (the `*` asks, not asserts).
pub fn status_range(total: u64) -> String {
    format!("bytes */{total}")
}

/// What one resumable response taught us about the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    /// Inclusive end of the prefix the server holds (`Range: bytes=0-N` →
    /// `Some(N)`); `None` = the server holds nothing (or said nothing).
    pub received_end: Option<u64>,
    /// `true` on a 2xx — the whole resource is in.
    pub complete: bool,
    /// The video id the FINAL answer's body carries.
    pub video_id: Option<String>,
}

/// Parse one resumable response: status + `Range` header + body.
pub fn parse_resume(
    status: &str,
    range_header: Option<&str>,
    body: &str,
) -> Result<Resume, String> {
    let complete = status.starts_with("2");
    let received_end = range_header.and_then(|h| {
        let v = h.trim().strip_prefix("bytes=")?.trim();
        let end = v.rsplit_once('-')?.1.trim();
        end.parse::<u64>().ok()
    });
    if complete {
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            id: Option<String>,
        }
        let id = serde_json::from_str::<Wire>(body).ok().and_then(|w| w.id);
        return Ok(Resume {
            received_end: None,
            complete: true,
            video_id: id,
        });
    }
    let incomplete = status.starts_with("308");
    if !incomplete {
        return Err(format!("unexpected resumable status: {status}"));
    }
    Ok(Resume {
        received_end,
        complete: false,
        video_id: None,
    })
}

/// Title sanitizer: the filename becomes the video title — path stripped,
/// whitespace collapsed, YouTube's 100-character cap honored, never empty.
/// The extension STAYS (the title names the file the user picked, not a
/// hand-typed program title).
pub fn sanitize_title(source_path: &str) -> String {
    let name = source_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source_path);
    let mut out = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 100 {
        out = out.chars().take(100).collect();
    }
    if out.is_empty() {
        out = "Upload".into();
    }
    out
}

/// The lane's configuration (adapter-level, never per-job).
#[derive(Debug, Clone)]
pub struct YoutubeConfig {
    /// The Google client (id + secret — the one-time sheet setup until
    /// IUM ships its own registered client).
    pub client: GoogleClient,
    /// The stored grant: access token + refresh token + expiry.
    pub token: TokenSet,
    /// `privacyStatus` (default PRIVATE — never public by accident).
    pub privacy: String,
    /// `snippet.categoryId` (default 22).
    pub category: String,
}

impl YoutubeConfig {
    /// The required-credential check: client id + client secret + token,
    /// each refusal naming the env var it expected. Sync + pure so the
    /// missing-path is testable without a process env.
    pub fn required(
        client_id: Option<&str>,
        client_secret: Option<&str>,
        token: Option<&str>,
    ) -> Result<(String, String, String), AdapterError> {
        let id = client_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AdapterError::AuthRequired("IUM_GOOGLE_CLIENT_ID not set".into()))?;
        let secret = client_secret
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AdapterError::AuthRequired("IUM_GOOGLE_CLIENT_SECRET not set".into()))?;
        let token = token
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AdapterError::AuthRequired("IUM_YOUTUBE_TOKEN not set".into()))?;
        Ok((id.to_string(), secret.to_string(), token.to_string()))
    }

    /// The pure constructor: complete parts in, config out. A token JSON
    /// that is not a token is an honest refusal (a pasted `ndus` cookie in
    /// the token field must never look like a signed-in lane).
    pub fn from_parts(
        client_id: &str,
        client_secret: &str,
        token_json: &str,
        privacy: &str,
        category: &str,
    ) -> Result<Self, AdapterError> {
        let (id, secret, token_json) =
            Self::required(Some(client_id), Some(client_secret), Some(token_json))?;
        let token = TokenSet::parse(&token_json).map_err(|e| {
            AdapterError::AuthRequired(format!(
                "the YouTube sign-in token is unusable — connect once again. ({e})"
            ))
        })?;
        Ok(Self {
            client: GoogleClient {
                client_id: id,
                client_secret: secret,
            },
            token,
            privacy: if privacy.trim().is_empty() {
                PRIVACY_DEFAULT.into()
            } else {
                privacy.trim().to_string()
            },
            category: if category.trim().is_empty() {
                CATEGORY_DEFAULT.into()
            } else {
                category.trim().to_string()
            },
        })
    }

    /// The env constructor (`IUM_GOOGLE_CLIENT_ID` + `IUM_GOOGLE_CLIENT_SECRET`
    /// + `IUM_YOUTUBE_TOKEN`; optional `IUM_YOUTUBE_PRIVACY` /
    ///   `IUM_YOUTUBE_CATEGORY`). The vault's apply_env fills all three.
    pub fn from_env() -> Result<Self, AdapterError> {
        let (id, secret, token) = Self::required(
            std::env::var("IUM_GOOGLE_CLIENT_ID").ok().as_deref(),
            std::env::var("IUM_GOOGLE_CLIENT_SECRET").ok().as_deref(),
            std::env::var("IUM_YOUTUBE_TOKEN").ok().as_deref(),
        )?;
        let privacy = std::env::var("IUM_YOUTUBE_PRIVACY").unwrap_or_default();
        let category = std::env::var("IUM_YOUTUBE_CATEGORY").unwrap_or_default();
        Self::from_parts(&id, &secret, &token, &privacy, &category)
    }
}

/// The session truth, mutable across confirms and reattaches.
#[derive(Default)]
struct SessionState {
    /// The resumable session URI (the journaled probe token's living form).
    session_uri: Option<String>,
    /// The video id the final answer carried (verify's subject).
    video_id: Option<String>,
    /// The access token the lane rides NOW (refreshed lazily; the stored
    /// grant keeps its refresh token — restarts stay honest).
    access_token: String,
    /// The current access token's absolute expiry (unix seconds). The
    /// refresh fires 60 s EARLY — before the wire refuses, never after.
    access_expires_at: Option<u64>,
}

/// The YouTube adapter — one connected account, one lane.
pub struct YoutubeAdapter {
    cfg: YoutubeConfig,
    http: Client,
    state: RwLock<SessionState>,
}

fn req_err(e: reqwest::Error) -> AdapterError {
    AdapterError::Transient(format!("transport: {e}"))
}

fn io_err(e: std::io::Error) -> AdapterError {
    AdapterError::Transient(format!("local io: {e}"))
}

impl YoutubeAdapter {
    /// Production constructor: the config from env, fresh token carried.
    pub fn new(cfg: YoutubeConfig) -> Self {
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
            state: RwLock::new(SessionState::default()),
        }
    }

    /// A fresh bearer header, refreshing lazily on expiry (the stored
    /// grant's refresh token is the durable credential; Google does not
    /// rotate it for installed-app clients, and the vault keeps the
    /// original — a restart re-reads it. Honest scope: an in-process
    /// ROTATED grant is not persisted; #85 documents that).
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
        let Some(rt) = self.cfg.token.refresh_token.clone() else {
            // No refresh grant: ride the stored access token while it
            // plausibly lives (an expiry we cannot judge is not one we
            // guess — the wire's 401 answers honestly on the next call).
            let st = self.state.read().expect("state lock");
            if self.cfg.token.expires_at.is_none() && !st.access_token.is_empty() {
                return Ok(st.access_token.clone());
            }
            if self.cfg.token.expires_at.is_none() {
                return Ok(self.cfg.token.access_token.clone());
            }
            return Err(AdapterError::AuthRequired(
                "the YouTube sign-in expired and carries no refresh token — connect once again"
                    .into(),
            ));
        };
        let fresh = self
            .cfg
            .client
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

    /// The read-only credential check (#81): `channels.list?mine=true` —
    /// a metadata read, never an upload session a Test button could orphan.
    pub async fn check_credentials(&self) -> Result<(), AdapterError> {
        let token = self.bearer().await?;
        let resp = self
            .http
            .get("https://www.googleapis.com/youtube/v3/channels")
            .query(&[("part", "id"), ("mine", "true")])
            .bearer_auth(&token)
            .send()
            .await
            .map_err(req_err)?;
        match resp.status() {
            StatusCode::OK => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(AdapterError::AuthRequired(format!(
                    "Google refused the YouTube check ({}): {}",
                    resp.status(),
                    resp.text()
                        .await
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect::<String>()
                )))
            }
            s if s.is_server_error() => Err(AdapterError::Transient(format!(
                "YouTube answered {s} for the credential check"
            ))),
            s => Err(AdapterError::Unknown(format!(
                "YouTube answered {s} for the credential check"
            ))),
        }
    }

    /// The source's size, checked against the journal (a source that
    /// changed underneath the job is Corruption, never silent loss).
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

    /// Ask the session what it holds (`Content-Range: bytes */total`).
    /// `Ok(None)` = the session URI is not in this process's state — the
    /// caller reattaches from the journaled token first.
    async fn session_status(&self, spec: &JobSpec) -> Result<Option<Resume>, AdapterError> {
        let uri = {
            let st = self.state.read().expect("state lock");
            st.session_uri.clone()
        };
        let Some(uri) = uri else {
            return Ok(None);
        };
        let total = self.source_len(spec).await?;
        let token = self.bearer().await?;
        let resp = self
            .http
            .put(&uri)
            .header(CONTENT_LENGTH, 0)
            .header("Content-Range", status_range(total))
            .body("")
            .bearer_auth(&token)
            .send()
            .await
            .map_err(req_err)?;
        let status = resp.status().as_u16().to_string();
        let range = resp
            .headers()
            .get("Range")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp.text().await.unwrap_or_default();
        parse_resume(&status, range.as_deref(), &body)
            .map(Some)
            .map_err(AdapterError::Unknown)
    }
}

/// The wire-byte counter over a seeked source — the same shape the S3 and
/// HTTP adapters ride (shared pattern #7).

#[async_trait]
impl TransferAdapter for YoutubeAdapter {
    fn name(&self) -> &'static str {
        LANE
    }

    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.source_len(spec).await?;
        let title = sanitize_title(&spec.source_path);
        let body = metadata_body(&title, &self.cfg.privacy, &self.cfg.category);
        let token = self.bearer().await?;
        let resp = self
            .http
            .post("https://www.googleapis.com/upload/youtube/v3/videos")
            .query(&[("uploadType", "resumable"), ("part", "snippet,status")])
            .header(CONTENT_TYPE, "application/json; charset=UTF-8")
            .header(
                CONTENT_LENGTH,
                HeaderValue::from_str(&body.len().to_string())
                    .expect("a decimal length is always a header value"),
            )
            .body(body)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(req_err)?;
        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    AdapterError::AuthRequired(format!(
                        "Google refused the session init ({status}): {}",
                        detail.chars().take(200).collect::<String>()
                    ))
                }
                StatusCode::TOO_MANY_REQUESTS => AdapterError::Throttled {
                    retry_after_ms: None,
                    message: format!("quota: {detail}"),
                },
                s if s.is_server_error() => AdapterError::Transient(format!("session init: {s}")),
                _ => AdapterError::Unknown(format!(
                    "session init: {status}: {}",
                    detail.chars().take(200).collect::<String>()
                )),
            });
        }
        let uri = resp
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| {
                AdapterError::Unknown("the session init answered without a Location".into())
            })?;
        {
            let mut st = self.state.write().expect("state lock");
            st.session_uri = Some(uri.clone());
            st.video_id = None;
        }
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(CHUNK),
            min_chunk_bytes: Some(MIN_CHUNK),
            max_parallel: Some(MAX_PARALLEL),
            session_token: Some(uri),
            uniform_block: None,
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
        let uri = self
            .state
            .read()
            .expect("state lock")
            .session_uri
            .clone()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no live session in this process — reattach from the journaled token".into(),
                )
            })?;

        // What does the server already hold? The status probe is cheap
        // (0 bytes) and is the session's OWN truth (308 + Range).
        let held: Resume = match self.session_status(spec).await? {
            Some(r) => r,
            None => {
                return Err(AdapterError::SessionLost(
                    "the session vanished — reattach from the journaled token".into(),
                ))
            }
        };
        if held.complete {
            // The whole resource is already in (a prior process finished
            // the last PUT and died before journalling). Confirm the span.
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.resume_from,
                adapter_token: held.video_id.clone(),
            });
        }
        let held_count = held.received_end.map(|e| e + 1).unwrap_or(0);
        let send_from = lease.resume_from.max(held_count);
        if send_from < lease.begin {
            // The session is BEHIND the plan (a reset session): the plan's
            // optimism loses to the session's truth (ADR-002 discipline).
            return Err(AdapterError::ExpiredSession(format!(
                "the session holds {} bytes but the plan starts chunk {} at {} — resume re-derives",
                held_count, lease.idx, lease.begin
            )));
        }
        if send_from >= lease.end {
            // The server already holds this whole segment.
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.resume_from,
                adapter_token: held.video_id.clone(),
            });
        }

        let end_incl = lease.end - 1;
        let range = content_range(send_from, end_incl, total);
        let token = self.bearer().await?;
        let file = tokio::fs::File::open(&spec.source_path)
            .await
            .map_err(io_err)?;
        let mut src = file;
        src.seek(std::io::SeekFrom::Start(send_from))
            .await
            .map_err(io_err)?;
        let span = lease.end - send_from;
        let stream = FramedRead::new(
            Counting {
                inner: src.take(span),
                progress: progress.clone(),
            },
            BytesCodec::new(),
        );
        let resp = self
            .http
            .put(&uri)
            .header(
                CONTENT_LENGTH,
                HeaderValue::from_str(&span.to_string())
                    .expect("a decimal length is always a header value"),
            )
            .header("Content-Range", range)
            .bearer_auth(&token)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
            .map_err(req_err)?;
        let status = resp.status().as_u16().to_string();
        let range_hdr = resp
            .headers()
            .get("Range")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp.text().await.unwrap_or_default();
        if status.starts_with('5') || status == "0" {
            return Err(AdapterError::Transient(format!(
                "YouTube answered {status} mid-chunk: {}",
                body.chars().take(120).collect::<String>()
            )));
        }
        let r =
            parse_resume(&status, range_hdr.as_deref(), &body).map_err(AdapterError::Unknown)?;
        if r.complete {
            if let Some(id) = r.video_id.clone() {
                self.state.write().expect("state lock").video_id = Some(id);
            }
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.resume_from,
                adapter_token: r.video_id.clone(),
            });
        }
        // 308: the server acknowledged the prefix it now holds. The WHOLE
        // prefix from the lease's resume_from is server-held (the Range IS
        // the acknowledgment), so the journal may count it.
        let held_now = r.received_end.map(|e| e + 1).unwrap_or(held_count);
        let confirmed = held_now
            .saturating_sub(lease.resume_from)
            .min(lease.end - lease.resume_from);
        if held_now >= lease.end {
            // The server holds the whole segment but the session is not
            // finalized — the confirm completes this chunk either way.
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.resume_from,
                adapter_token: r.video_id.clone(),
            });
        }
        if confirmed == 0 {
            return Err(AdapterError::Transient(format!(
                "the session took no bytes this attempt (server holds {held_now})"
            )));
        }
        Err(AdapterError::Transient(format!(
            "the session was interrupted mid-chunk — the server holds {held_now} bytes and the retry resumes from there"
        )))
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let video_id = self.state.read().expect("state lock").video_id.clone();
        let Some(id) = video_id else {
            return Ok(VerifyReport {
                ok: false,
                detail: Some(
                    "no video id was captured — the upload never reached its final answer".into(),
                ),
            });
        };
        let _ = spec;
        let token = self.bearer().await?;
        let resp = self
            .http
            .get("https://www.googleapis.com/youtube/v3/videos")
            .query(&[("part", "status"), ("id", id.as_str())])
            .bearer_auth(&token)
            .send()
            .await
            .map_err(req_err)?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "the verify read failed ({status}): {}",
                    body.chars().take(160).collect::<String>()
                )),
            });
        }
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            items: Vec<Item>,
        }
        #[derive(serde::Deserialize)]
        struct Item {
            #[serde(default)]
            status: Option<St>,
        }
        #[derive(serde::Deserialize)]
        struct St {
            #[serde(default)]
            upload_status: Option<String>,
        }
        let parsed: Wire = serde_json::from_str(&body)
            .map_err(|e| AdapterError::Unknown(format!("verify body: {e}")))?;
        let upload = parsed
            .items
            .first()
            .and_then(|i| i.status.as_ref())
            .and_then(|s| s.upload_status.clone())
            .unwrap_or_default();
        let ok = matches!(upload.as_str(), "uploaded" | "processed");
        Ok(VerifyReport {
            ok,
            detail: Some(format!(
                "YouTube holds video {id} (uploadStatus={upload}). The API offers no server-side \
                 checksum — the lane verifies existence + status, honest-by-capability."
            )),
        })
    }

    async fn reattach(&self, _endpoint: &str, token: &str) -> Result<(), AdapterError> {
        // The journaled token IS the session URI. Never CREATE a session —
        // only adopt the journaled one and re-learn what the server holds.
        {
            let mut st = self.state.write().expect("state lock");
            st.session_uri = Some(token.to_string());
            st.video_id = None;
        }
        Ok(())
    }
}

/// The wire-byte counter over a seeked source — the same shape the S3 and
/// HTTP adapters ride (shared pattern #7).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_is_the_default_and_the_body_says_what_it_means() {
        assert_eq!(PRIVACY_DEFAULT, "private");
        let body = metadata_body("My Video", PRIVACY_DEFAULT, CATEGORY_DEFAULT);
        assert!(body.contains("\"privacyStatus\":\"private\""));
        assert!(body.contains("\"title\":\"My Video\""));
    }

    #[test]
    fn titles_come_from_filenames_and_never_exceed_100_chars() {
        assert_eq!(sanitize_title("/home/u/My Clip.mp4"), "My Clip.mp4");
        assert_eq!(sanitize_title("C:\\vids\\a b.wmv"), "a b.wmv");
        assert_eq!(sanitize_title(""), "Upload");
        assert_eq!(sanitize_title(&"x".repeat(250)).chars().count(), 100);
    }

    #[test]
    fn the_lane_declares_its_protocol() {
        assert_eq!(LANE, "youtube");
        assert_eq!(CHUNK, 8 * 1024 * 1024);
        assert_eq!(CHUNK % MIN_CHUNK, 0, "256 KiB grain");
        assert_eq!(MAX_PARALLEL, 1);
    }

    #[test]
    fn required_refuses_each_missing_piece_by_name() {
        let e = YoutubeConfig::required(None, None, None).unwrap_err();
        assert!(e.to_string().contains("IUM_GOOGLE_CLIENT_ID"));
        let e = YoutubeConfig::required(Some("c"), None, None).unwrap_err();
        assert!(e.to_string().contains("IUM_GOOGLE_CLIENT_SECRET"));
        let e = YoutubeConfig::required(Some("c"), Some("s"), None).unwrap_err();
        assert!(e.to_string().contains("IUM_YOUTUBE_TOKEN"));
    }

    #[test]
    fn a_non_token_refuses_loudly() {
        let e = YoutubeConfig::from_parts("c", "s", "not-json", "", "").unwrap_err();
        assert!(matches!(e, AdapterError::AuthRequired(_)));
    }

    #[test]
    fn ranges_speak_the_resumable_language() {
        assert_eq!(content_range(0, 7, 100), "bytes 0-7/100");
        assert_eq!(status_range(100), "bytes */100");
    }

    #[test]
    fn a_308_names_the_prefix_and_a_200_names_the_video() {
        let r = parse_resume("308", Some("bytes=0-8388607"), "").unwrap();
        assert_eq!(r.received_end, Some(8_388_607));
        assert!(!r.complete);
        let r = parse_resume("200", None, "{\"id\":\"v1\"}").unwrap();
        assert!(r.complete);
        assert_eq!(r.video_id.as_deref(), Some("v1"));
        assert!(parse_resume("400", None, "{}").is_err());
    }
}
