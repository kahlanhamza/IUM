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
//!   journaled);
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
//!   it never saw), otherwise → `Transient` (retries stay counted).
//! * verify is a **read-only** HEAD (`offset == length == source_size`) —
//!   TUS needs no commit gate (contrast ADR-010's S3 finalize-if-needed);
//! * tusd v2.10.1 offers no `checksum` extension — not used (verify is
//!   offset-based; per-request checksums are a future negotiation).
//!
//! Sessions are keyed by `spec.endpoint` (the creation URL), journaled via
//! the probe event (ADR-002); `restore_session` is the reattach path —
//! identical shape to S3. The empty `[(0, 0)]` chunk (issue #5) confirms via
//! HEAD instead of a pointless empty PATCH — still a real server round-trip.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt};
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

/// TUS 1.0.0 client. No credentials by design (tusd-style servers); auth
/// headers are a future negotiation when a real deployment needs them.
pub struct TusAdapter {
    client: reqwest::Client,
    sessions: Mutex<HashMap<String, String>>,
}

impl TusAdapter {
    /// A fresh adapter with its own HTTP client and an EMPTY session map —
    /// sessions are restored from the journal via `reattach` (#12), never
    /// guessed.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .expect("reqwest client builds"),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The upload URL for a job (the journaled probe event's session token).
    pub fn session_for(&self, endpoint: &str) -> Option<String> {
        self.sessions.lock().unwrap().get(endpoint).cloned()
    }

    /// Reattach a fresh adapter instance to an existing upload (the host
    /// replays the journaled probe event — ADR-002).
    pub fn restore_session(&self, endpoint: &str, location: String) {
        self.sessions
            .lock()
            .unwrap()
            .insert(endpoint.to_string(), location);
    }

    fn io_err(e: std::io::Error) -> AdapterError {
        AdapterError::Transient(format!("source read failed: {e}"))
    }

    /// `HEAD <loc>` → `(Upload-Length, Upload-Offset)`. The offset is the
    /// server's durable truth; the length is what creation declared.
    async fn head(&self, location: &str) -> Result<(u64, u64), AdapterError> {
        let resp = self
            .client
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

    /// One PATCH from `offset`, streaming `[offset, end)` from the source.
    /// `409` is not an `AdapterError` — it is the protocol's crash-recovery
    /// signal and the caller (the realign dance) must see it as data.
    async fn patch(
        &self,
        spec: &JobSpec,
        location: &str,
        offset: u64,
        end: u64,
        progress: &ProgressReporter,
    ) -> Result<u64, PatchFailure> {
        let span = end - offset;
        let mut file = tokio::fs::File::open(&spec.source_path)
            .await
            .map_err(Self::io_err)
            .map_err(PatchFailure::Adapter)?;
        file.seek(std::io::SeekFrom::Start(offset))
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

        let resp = match self
            .client
            .patch(location)
            .header("Tus-Resumable", TUS_VERSION)
            .header("Upload-Offset", offset.to_string())
            .header("Content-Type", TUS_CONTENT_TYPE)
            .header("Content-Length", span.to_string())
            .body(body)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                // Transport failure is ambiguous by itself: a real network
                // blip (→ Transient, retry) and @tus/server's early-reject
                // socket close (#33/#37 — the 404/409 never reaches the
                // client while the body is still streaming) look identical
                // to hyper. Ask the server before declaring — its answer,
                // not the socket's mood, classifies (verify-before-declare,
                // for error paths too):
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
                return Err(match self.head(location).await {
                    Err(AdapterError::SessionLost(_)) => {
                        PatchFailure::Adapter(AdapterError::SessionLost(format!(
                            "PATCH transport-failed and HEAD confirms the session is gone: {e}"
                        )))
                    }
                    Ok((_, server_offset)) if server_offset != offset => PatchFailure::Conflict,
                    _ => {
                        PatchFailure::Adapter(AdapterError::Transient(format!("PATCH failed: {e}")))
                    }
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
            423 => Err(PatchFailure::Adapter(AdapterError::Transient(
                "423 Locked: upload busy".into(),
            ))),
            s if s >= 500 => Err(PatchFailure::Adapter(AdapterError::Transient(format!(
                "PATCH -> {s}"
            )))),
            s => Err(PatchFailure::Adapter(AdapterError::Unknown(format!(
                "PATCH -> {s}"
            )))),
        }
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

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
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
                    resumable: true,
                    max_chunk_bytes: None,
                    // Issue #13: TUS is ONE sequential-offset resource.
                    // Concurrent PATCHes to different offsets 409-realign
                    // into Unresumable — the protocol itself caps parallel
                    // uploads at 1 per session, so the adapter DECLARES it
                    // (ADR-011 channel) and the planner obeys.
                    max_parallel: Some(1),
                    session_token: Some(location),
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

    /// Reattach (issue #12): replay the journaled probe event's creation URL
    /// into the session map. Pure in-memory restore — no network, no new
    /// POST creation (that would start a second upload resource and orphan
    /// the first).
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.restore_session(endpoint, token.to_string());
        Ok(())
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let location = self.session_for(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(
                "no tus session for endpoint; restore from the journaled probe event".into(),
            )
        })?;

        // The empty span (issue #5's [(0, 0)] chunk): confirm via HEAD — a
        // real server round-trip, zero pointless empty PATCHes.
        if lease.end == lease.resume_from {
            let (_, server_offset) = self.head(&location).await?;
            if server_offset < lease.resume_from {
                return Err(AdapterError::Unresumable(format!(
                    "server offset {server_offset} behind journaled {}",
                    lease.resume_from
                )));
            }
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: 0,
                adapter_token: Some(server_offset.to_string()),
            });
        }

        // Issue #30 wire honesty: snapshot before the first PATCH so the
        // rejected attempt's flush can be reclassified exactly.
        let first_attempt_from = progress.moved();
        let sent = self
            .patch(spec, &location, lease.resume_from, lease.end, progress)
            .await;
        let final_offset = match sent {
            Ok(offset) => offset,
            // 409 = the server is AHEAD of the journal: an in-flight PATCH
            // was killed after the server acked but before the journal write.
            // Absorb the drift: HEAD, re-PATCH from the server offset. The
            // drifted bytes were server-acked — counting them in THIS call's
            // confirm is honest and makes the journal converge to the truth.
            Err(PatchFailure::Conflict) => {
                // The server decided the 409 on the request HEAD and read
                // none of the body; whatever this attempt flushed into the
                // socket before the rejection surfaced crossed the wire and
                // was persisted NOWHERE (server-verified: the offset never
                // moves on a rejected PATCH). Its size is a scheduling race,
                // not a protocol constant (issue #30) — so it is MEASURED
                // waste, and the realign's bytes below stay honest movement.
                let (_, server_offset) = self.head(&location).await?;
                if server_offset < lease.resume_from {
                    return Err(AdapterError::Unresumable(format!(
                        "server offset {server_offset} behind journaled {} — the server lost confirmed bytes",
                        lease.resume_from
                    )));
                }
                if server_offset > lease.end {
                    return Err(AdapterError::Unknown(format!(
                        "server offset {server_offset} beyond this chunk's end {}",
                        lease.end
                    )));
                }
                // Exactly one realign attempt: another 409 here means the
                // server moved under a single-worker lease — fail loudly.
                let realigned = self
                    .patch(spec, &location, server_offset, lease.end, progress)
                    .await
                    .map_err(|f| match f {
                        PatchFailure::Conflict => AdapterError::Unknown(
                            "409 again after realign — server moved under the lease".into(),
                        ),
                        PatchFailure::Adapter(e) => e,
                    })?;
                // THE WASTE RECKONING — settled arithmetic, not an instant
                // snapshot (issue #37): the rejected attempt's last chunks
                // can still land in `moved()` after `patch()` returns (the
                // body is counted on hyper's connection task, and a store
                // can drain after the caller resumes — an 8 KiB chunk
                // flipped CI's exact assert into a rare flake). Classify
                // AFTER the realign settles: everything this call pushed
                // BEYOND the re-PATCH's server-acked tail is, by
                // construction, the rejected attempt's flush — persisted
                // nowhere, exactly waste, whenever its count landed.
                let tail = realigned - server_offset;
                let pushed = progress.moved() - first_attempt_from;
                progress.waste(pushed.saturating_sub(tail));
                realigned
            }
            Err(PatchFailure::Adapter(e)) => return Err(e),
        };

        if final_offset < lease.resume_from {
            return Err(AdapterError::Unresumable(format!(
                "server acked offset {final_offset} behind the lease start {}",
                lease.resume_from
            )));
        }
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: final_offset - lease.resume_from,
            adapter_token: Some(final_offset.to_string()),
        })
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        // Read-only integrity gate: the server holds exactly source_size
        // bytes. No finalize step exists in TUS (contrast ADR-010).
        let location = self.session_for(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(
                "no tus session for endpoint; restore from the journaled probe event".into(),
            )
        })?;
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
