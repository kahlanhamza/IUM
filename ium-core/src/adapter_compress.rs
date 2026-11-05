//! The `compress` pseudo-adapter (issue #53): entropy-gated zstd transfer
//! mode over ANY resumable lane — Cause-D byte reduction for the 3 KB/s
//! regime, with honest destination semantics.
//!
//! ## Shape (design note §3, D1–D6)
//!
//! * **Accepted jobs compress ONCE** to `<source>.zst` (checksummed frame,
//!   level journaled; a present archive is decode-verified against a freshly
//!   computed source hash and reused — resume-fast) and the ARCHIVE is the
//!   wrapped lane's source: **every drill property (kill-9, zero re-upload,
//!   409-realign, verify) is inherited by the archive bytes unchanged.**
//! * **The single-chunk shape (D2):** `min_chunk_bytes = Some(source_size)`
//!   forces the planner to ONE piece spanning the whole source (resplit is
//!   floor-blocked; replans keep n = 1) — the manager's pool plans the source
//!   span, the adapter drives the whole archive as one resumable upload, and
//!   the confirm is a journaled linear projection (archive delta ×
//!   source/archive, clamped; exact completion when the archive is fully
//!   acked). Wire truth lives in the `ProgressReporter` (archive bytes).
//! * **The manifest** (the probe event's session token JSON) carries mode,
//!   level, source_sha256, archive_sha256, both sizes, ratio, and the inner
//!   token — both hashes journaled, the destination honesty the UI reads
//!   ("stored compressed, N% saved"; the stored-object name is the host's
//!   mapping, W13).
//! * **Verify = three gates** (D4): inner HEAD byte-exact on the archive +
//!   zstd decode (checksum-validated, W4) + logical source sha256 — a
//!   corrupted archive fails verify loudly.
//! * **Decline paths are plain jobs** (D5): gate decline (`sample_ratio`
//!   journaled — the honest "a USB stick wins" line), `IUM_NO_COMPRESS=1`
//!   kill-switch (mirroring #48's instrument channel), and the #34
//!   disk-space check (statvfs, unix; cfg'd out on windows loudly).
//! * **Cleanup (D6, with the recorded deviation):** the manager finalizes
//!   BEFORE the verify gate, so cleanup lives at verify-SUCCESS (all three
//!   gates passed ⇒ the archive is no longer needed). Failed/Cancel/failed-
//!   verify KEEP it — the #24 repair path re-verifies, cancel-then-resume is
//!   a supported flow (#15), and the archive IS the transfer's resumable
//!   state; a fresh job with the same source reuses it.
//!
//! Opt-in by construction: the host registers `CompressAdapter` and sets
//! `spec.adapter = "compress"`; no JobSpec change, zero manager diff.

use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::compress::{compress_source, entropy_gate, sample_source, verify_archive, GateDecision};
use crate::JobSpec;

/// The staging archive for a compress job: `<source_path>.zst`.
pub fn staging_archive(spec: &JobSpec) -> PathBuf {
    PathBuf::from(format!("{}.zst", spec.source_path))
}

/// The job's transfer mode, fixed at probe (the manager drives chunks only
/// after probe/reattach — the mode is already known). Cloned out of the
/// mutex before every await (Send futures; the fec.rs lesson).
#[derive(Clone)]
enum Mode {
    /// Upload the archive through the inner lane.
    Accepted {
        spec_z: JobSpec,
        archive_size: u64,
        source_size: u64,
        source_sha256: String,
    },
    /// Plain job — the inner lane sees the ORIGINAL spec.
    Declined,
}

/// The entropy-gated zstd adapter. Construct with the wrapped lane, register
/// under its name (`"compress"`).
pub struct CompressAdapter {
    inner: Arc<dyn TransferAdapter>,
    lane_name: &'static str,
    mode: Mutex<Option<Mode>>,
}

impl CompressAdapter {
    /// Wrap one resumable lane (single archive session — no factory needed;
    /// the W13 endpoint collision does not arise with one session).
    pub fn new(inner: Arc<dyn TransferAdapter>) -> Self {
        let lane_name = inner.name();
        CompressAdapter {
            inner,
            lane_name,
            mode: Mutex::new(None),
        }
    }

    /// The encode level: the issue's "CPU cores vs measured bearer" — the
    /// wire-bound regime has CPU to burn; cores scale the effort, 19 caps it.
    fn level() -> i32 {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        (3 + 3 * cores as i32).min(19)
    }

    /// #34 discipline: the staging volume must hold the archive. unix-only
    /// (statvfs); windows has no libc equivalent here — cfg'd out loudly.
    #[cfg(unix)]
    fn disk_free(path: &std::path::Path) -> Option<u64> {
        let cpath = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(cpath.as_ptr(), &mut st) } != 0 {
            return None;
        }
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(not(unix))]
    fn disk_free(_path: &std::path::Path) -> Option<u64> {
        None // honest absence: no check on this platform (journaled as-is)
    }

    async fn decline(
        &self,
        spec: &JobSpec,
        why: &'static str,
        ratio: serde_json::Value,
    ) -> Result<ProbeReport, AdapterError> {
        let rep = self.inner.probe(spec).await?;
        let token = json!({
            "mode": "declined",
            "why": why,
            "sample_ratio": ratio,
            "inner": rep.session_token.clone().unwrap_or_default(),
        });
        *self.mode.lock().unwrap() = Some(Mode::Declined);
        Ok(ProbeReport {
            resumable: rep.resumable,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: rep.max_parallel,
            session_token: Some(token.to_string()),
            concatenation: false,
        })
    }
}

#[async_trait]
impl TransferAdapter for CompressAdapter {
    fn name(&self) -> &'static str {
        "compress"
    }

    /// Badges pass through from the wrapped lane (honesty is the lane's).
    fn capabilities(&self) -> Capability {
        self.inner.capabilities()
    }

    /// The entropy gate → either the archive path (D2/D3/D4) or a plain job
    /// with the reason journaled (D5).
    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        if std::env::var("IUM_NO_COMPRESS").as_deref() == Ok("1") {
            return self
                .decline(spec, "IUM_NO_COMPRESS", serde_json::Value::Null)
                .await;
        }
        // #34 discipline: the volume must hold the archive before encoding.
        let zst = staging_archive(spec);
        if let Some(free) = Self::disk_free(&zst) {
            if free < spec.source_size + spec.source_size / 10 {
                return self.decline(spec, "disk", serde_json::Value::Null).await;
            }
        }

        let samples = sample_source(Path::new(&spec.source_path), spec.source_size)
            .map_err(|e| AdapterError::Transient(format!("compress sampling: {e}")))?;
        let gate: GateDecision = entropy_gate(&samples);
        if let GateDecision::Decline {
            projected_ratio,
            why,
        } = gate
        {
            return self.decline(spec, why, json!(projected_ratio)).await;
        }

        // The archive: reuse a present one only if it decodes to the freshly
        // computed source hash (resume-fast; never trusts a stale file).
        let level = Self::level();
        let source = Path::new(&spec.source_path);
        let source_sha256 = crate::compress::hash_file(source)
            .map_err(|e| AdapterError::Transient(format!("compress hashing: {e}")))?;
        let archive_size = if zst.exists() {
            match verify_archive(&zst, &source_sha256, spec.source_size) {
                Ok(true) => std::fs::metadata(&zst)
                    .map_err(|e| AdapterError::Transient(e.to_string()))?
                    .len(),
                _ => {
                    compress_source(source, &zst, level)
                        .map_err(|e| AdapterError::Transient(format!("compress encode: {e}")))?
                        .0
                }
            }
        } else {
            compress_source(source, &zst, level)
                .map_err(|e| AdapterError::Transient(format!("compress encode: {e}")))?
                .0
        };
        let archive_sha256 = crate::compress::hash_file(&zst)
            .map_err(|e| AdapterError::Transient(format!("compress hashing: {e}")))?;
        let source_size = spec.source_size;
        let ratio = source_size as f64 / archive_size.max(1) as f64;

        let spec_z = JobSpec {
            source_path: zst.to_string_lossy().into_owned(),
            source_size: archive_size,
            adapter: self.lane_name.to_string(),
            endpoint: spec.endpoint.clone(),
        };
        let rep = self.inner.probe(&spec_z).await?;
        let token = json!({
            "mode": "compressed",
            "level": level,
            "source_sha256": source_sha256,
            "archive_sha256": archive_sha256,
            "source_size": source_size,
            "archive_size": archive_size,
            "ratio": ratio,
            "inner": rep.session_token.clone().unwrap_or_default(),
        });
        *self.mode.lock().unwrap() = Some(Mode::Accepted {
            spec_z,
            archive_size,
            source_size,
            source_sha256,
        });
        Ok(ProbeReport {
            resumable: rep.resumable,
            max_chunk_bytes: None,
            min_chunk_bytes: Some(source_size),
            max_parallel: Some(1),
            session_token: Some(token.to_string()),
            concatenation: false,
        })
    }

    /// Accepted: drive the WHOLE archive through the inner lane; confirm is
    /// the journaled linear projection (D3) — exact at archive completion.
    /// Declined: the lease passes through unchanged.
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let mode = self.mode.lock().unwrap().clone();
        match mode {
            None => Err(AdapterError::SessionLost(
                "compress: drive before probe/reattach".into(),
            )),
            Some(Mode::Declined) => self.inner.upload_chunk(spec, lease, progress).await,
            Some(Mode::Accepted {
                spec_z,
                archive_size,
                source_size,
                ..
            }) => {
                let lease_z = Lease {
                    idx: lease.idx,
                    begin: 0,
                    end: archive_size,
                    resume_from: 0,
                    attempt: lease.attempt,
                };
                let c = self.inner.upload_chunk(&spec_z, &lease_z, progress).await?;
                let done = c.bytes_confirmed >= archive_size;
                let remaining = lease.end - lease.resume_from;
                let confirmed = if done {
                    remaining
                } else {
                    (((c.bytes_confirmed as u128 * source_size as u128)
                        / (archive_size.max(1) as u128)) as u64)
                        .min(remaining)
                };
                Ok(ChunkConfirm {
                    idx: lease.idx,
                    bytes_confirmed: confirmed,
                    adapter_token: c.adapter_token,
                })
            }
        }
    }

    /// Three gates (D4): the inner's HEAD on the archive, the zstd checksum
    /// decode, and the logical source sha256 — a corrupted archive fails
    /// loudly. Declined: pass through.
    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let mode = self.mode.lock().unwrap().clone();
        match mode {
            None => Ok(VerifyReport {
                ok: false,
                detail: Some("compress: nothing probed — nothing to verify".into()),
            }),
            Some(Mode::Declined) => self.inner.verify(spec).await,
            Some(Mode::Accepted {
                spec_z,
                archive_size,
                source_size,
                source_sha256,
            }) => {
                let head = self.inner.verify(&spec_z).await?;
                if !head.ok {
                    return Ok(VerifyReport {
                        ok: false,
                        detail: Some(format!(
                            "compress: archive HEAD failed on the lane: {}",
                            head.detail.unwrap_or_default()
                        )),
                    });
                }
                let zst = staging_archive(spec);
                match verify_archive(&zst, &source_sha256, source_size) {
                    Ok(true) => {
                        // All three gates passed — the archive is no longer
                        // needed (verify-success IS the cleanup point; see
                        // finalize's doc for why it cannot be there).
                        if let Err(e) = std::fs::remove_file(&zst) {
                            tracing::warn!("compress archive cleanup failed (best-effort): {e}");
                        }
                        let saved = 100.0
                            * (1.0 - archive_size as f64 / (source_size as f64).max(1.0));
                        Ok(VerifyReport {
                            ok: true,
                            detail: Some(format!(
                                "compress: archive HEAD byte-exact ({archive_size} B) + zstd decode checksum ok + logical sha256 ok — stored as <name>.zst, {saved:.1}% saved; destination decode is the receiver's job"
                            )),
                        })
                    }
                    Ok(false) => Ok(VerifyReport {
                        ok: false,
                        detail: Some(
                            "compress: archive decoded but the logical hash/size MISMATCHES the manifest".into(),
                        ),
                    }),
                    Err(e) => Ok(VerifyReport {
                        ok: false,
                        detail: Some(format!(
                            "compress: archive decode FAILED (checksum gate): {e}"
                        )),
                    }),
                }
            }
        }
    }

    /// Replay the inner token (the manifest carries it — ADR-002).
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        let v: serde_json::Value = serde_json::from_str(token)
            .map_err(|e| AdapterError::Unresumable(format!("compress token: {e}")))?;
        let inner = v["inner"]
            .as_str()
            .ok_or_else(|| AdapterError::Unresumable("compress token missing inner".into()))?;
        if inner.is_empty() {
            return Ok(());
        }
        self.inner.reattach(endpoint, inner).await
    }

    /// No protocol finalization — and NO cleanup here: the manager calls
    /// finalize BEFORE the verify gate, and verify's decode gate needs the
    /// archive bytes. The removal happens at verify-SUCCESS instead (all
    /// three gates passed ⇒ the archive is no longer needed; a FAILED verify
    /// keeps it for the #24 repair path's re-verification).
    async fn finalize(&self, _spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        Ok(None)
    }
}
