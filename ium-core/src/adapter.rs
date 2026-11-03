//! Transfer adapters — one module per protocol family, behind capability
//! flags (pattern #7: FDM's `AbstractDownloadModule` registry shape,
//! rebuilt for uploads).

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::chunk::Lease;
use crate::JobSpec;

/// Bitflags a protocol adapter declares honestly — the UI turns these into
/// the Resumable ✅ / One-shot ⚠️ badges (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capability(u8);

impl Capability {
    /// The endpoint can resume an interrupted transfer into an existing
    /// session (S3 multipart, TUS, HTTP range). Drives the honest badge.
    pub const RESUMABLE: u8 = 1 << 0;
    /// The endpoint accepts independently-sized chunks — the planner may
    /// re-split at probe boundaries (#8) instead of a fixed grid.
    pub const DYNAMIC_CHUNKS: u8 = 1 << 1;
    /// The endpoint tolerates a racing tail request (spec §4.2's tail-race
    /// optimization: the last chunk re-requested while finishing).
    pub const TAIL_RACE: u8 = 1 << 2;

    /// Build a capability set from raw bits (adapters declare at construction).
    pub const fn new(bits: u8) -> Self {
        Capability(bits)
    }
    /// Does this set contain every bit in `bit`?
    pub const fn has(self, bit: u8) -> bool {
        self.0 & bit == bit
    }
    /// Raw bits (journal serialization, tests).
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Human-readable badge for the journal/UI.
    pub fn badge(self) -> &'static str {
        if self.has(Capability::RESUMABLE) {
            "resumable"
        } else {
            "one_shot"
        }
    }
}

/// What a probe learned about the endpoint before the first byte moves.
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// Does the endpoint let us resume an existing session?
    pub resumable: bool,
    /// Server-side size limit for one request/chunk, if it declares one.
    pub max_chunk_bytes: Option<u64>,
    /// How many workers may upload CONCURRENTLY against one session, if the
    /// protocol constrains it (issue #13, same declaration channel as
    /// `max_chunk_bytes` per ADR-011: adapter declares, planner obeys).
    /// `None` = no declared limit. TUS is ONE sequential-offset resource and
    /// declares `Some(1)`; S3 parts are independent and declare `None`.
    pub max_parallel: Option<u32>,
    /// Opaque session descriptor to carry across confirms.
    pub session_token: Option<String>,
}

/// A chunk that completed with a server acknowledgment.
#[derive(Debug, Clone)]
pub struct ChunkConfirm {
    /// Index of the chunk this confirm belongs to (journal key).
    pub idx: u32,
    /// Bytes the SERVER confirmed BY THIS CALL, counted over the transferred
    /// segment `[lease.resume_from, lease.end)` (spec rule #1: never client
    /// optimism). The manager composes this with the chunk's prior
    /// confirmations into the absolute in-chunk total.
    pub bytes_confirmed: u64,
    /// Protocol token to reuse on reattach (part number, offset, cursor).
    pub adapter_token: Option<String>,
}

/// Result of the end-of-job integrity gate (pattern #10).
#[derive(Debug, Clone)]
pub struct VerifyReport {
    /// `true` only when the adapter confirmed the destination holds exactly
    /// the source's bytes (or its own ADR-010-equivalent gate passed).
    pub ok: bool,
    /// Human-readable evidence: what was checked, or why it failed — this
    /// goes into the journal's `verify_failed` event verbatim.
    pub detail: Option<String>,
}

/// Live byte counter for one in-flight `upload_chunk` call.
///
/// This is TELEMETRY, never truth: the journal keeps only server
/// confirmations (`bytes_confirmed`), while this counter reports what the
/// adapter pushed onto the wire — exactly what slow-chunk eviction (spec
/// §4.2) and the future per-chunk speed inspector (spec §8) need.
///
/// Segment-relative: counts bytes moved for `[lease.resume_from, lease.end)`.
/// A fresh reporter is handed to every `upload_chunk` call.
#[derive(Clone, Default)]
pub struct ProgressReporter {
    bytes: Arc<AtomicU64>,
    wasted: Arc<AtomicU64>,
}

impl ProgressReporter {
    /// A fresh counter at zero — one per `upload_chunk` call.
    pub fn new() -> Self {
        Self::default()
    }
    /// Record `n` more bytes handed to the transport. Cheap, non-blocking —
    /// adapters call this on the hot path.
    pub fn add(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }
    /// Bytes moved so far in this call.
    pub fn moved(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    /// Reclassify `n` of the already-counted [`ProgressReporter::moved`]
    /// bytes as WASTED (issue #30): they crossed the wire, but the server
    /// persisted none of them — a rejected 409 PATCH's socket flush, whose
    /// size is a client↔server scheduling race (0–48 KiB quiet, 192 KiB on a
    /// loaded runner; nightly run 41618615182), not a protocol constant.
    /// Waste stays in [`ProgressReporter::moved`] — it is real wire time and
    /// eviction/resplit must measure it — but wire-accounting consumers can
    /// now separate persisted movement from refused flush exactly:
    /// `moved() − wasted()` == bytes the server actually took. Never call
    /// this for bytes a later attempt re-sent successfully; those are
    /// persisted movement, not waste.
    pub fn waste(&self, n: u64) {
        debug_assert!(
            n <= self.bytes.load(Ordering::Relaxed),
            "waste reclassifies already-counted bytes"
        );
        self.wasted.fetch_add(n, Ordering::Relaxed);
    }
    /// Bytes counted in [`ProgressReporter::moved`] that the server never
    /// persisted (see [`ProgressReporter::waste`]).
    pub fn wasted(&self) -> u64 {
        self.wasted.load(Ordering::Relaxed)
    }
}

impl std::fmt::Debug for ProgressReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressReporter")
            .field("moved", &self.moved())
            .field("wasted", &self.wasted())
            .finish()
    }
}

/// Errors adapters return, already classified for the retry policy.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// Temporary failure (network blip, 5xx) — the retry policy backs off
    /// and re-attempts the same lease.
    #[error("transient: {0}")]
    Transient(String),
    /// The endpoint asked us to slow down (429). Carries the server's own
    /// `Retry-After` when it declared one — honored before local backoff.
    #[error("throttled: {message}")]
    Throttled {
        /// Server-declared delay in milliseconds, if any.
        retry_after_ms: Option<u64>,
        /// Human-readable context (status line, body excerpt).
        message: String,
    },
    /// The session existed but the endpoint expired it — recoverable via
    /// `reattach` from the journaled token (Action::ExpiredSession).
    #[error("expired session: {0}")]
    ExpiredSession(String),
    /// The session is gone from the adapter's memory — recoverable via
    /// `reattach` replaying the journaled probe token (#12).
    #[error("session lost: {0}")]
    SessionLost(String),
    /// The endpoint cannot resume this session at all (e.g. TUS 404 on a
    /// creation URL whose server offset is behind the journal) — the job
    /// fails honestly instead of re-uploading from zero silently.
    #[error("unresumable: {0}")]
    Unresumable(String),
    /// The source or the protocol state changed underneath the job
    /// (file resized, session moved). Never retried — the facts changed.
    #[error("corruption: {0}")]
    Corruption(String),
    /// Credentials are missing or rejected — user action required.
    #[error("auth required: {0}")]
    AuthRequired(String),
    /// Anything the classification cannot name honestly.
    #[error("unknown: {0}")]
    Unknown(String),
}

impl AdapterError {
    /// The retry-policy classification of this error (spec §5's taxonomy:
    /// which failures are retryable, which terminate the job).
    pub fn kind(&self) -> crate::retry::ErrorKind {
        use crate::retry::ErrorKind;
        match self {
            AdapterError::Transient(_) => ErrorKind::Transient,
            AdapterError::Throttled { .. } => ErrorKind::Throttled,
            AdapterError::ExpiredSession(_) => ErrorKind::ExpiredSession,
            AdapterError::SessionLost(_) => ErrorKind::SessionLost,
            AdapterError::Unresumable(_) => ErrorKind::Unresumable,
            AdapterError::Corruption(_) => ErrorKind::Corruption,
            AdapterError::AuthRequired(_) => ErrorKind::AuthRequired,
            AdapterError::Unknown(_) => ErrorKind::Unknown,
        }
    }
}

/// One protocol family. Implementations must be Send + Sync; the manager may
/// call `upload_chunk` concurrently from multiple worker tasks (Sprint 1).
///
/// Implementations read the bytes they need directly from `spec.source_path`
/// at the offsets the lease dictates — the core never buffers whole files.
#[async_trait]
pub trait TransferAdapter: Send + Sync {
    /// Registry key ("s3_multipart", "tus", "http_range", ...).
    fn name(&self) -> &'static str;

    /// Honest capability declaration → badges.
    fn capabilities(&self) -> Capability;

    /// Pre-flight: validate endpoint, discover resume support, session token.
    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError>;

    /// Upload one leased chunk, returning only server-confirmed bytes.
    /// `lease.resume_from` (absolute offset) is where the transfer must start;
    /// the range to move is `[resume_from, lease.end)`.
    ///
    /// `progress` receives wire-byte counts as the body streams; implementations
    /// SHOULD report regularly so the supervisor can evict stalled chunks
    /// (spec §4.2) instead of waiting for a timeout to guess.
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError>;

    /// End-of-job integrity gate (pattern #10, ADR-004). **Required, no
    /// default** — verify-before-declare is not opt-in. An adapter that
    /// cannot truly verify (e.g. a protocol without server-side metadata)
    /// must still implement this and say so honestly, e.g. via
    /// `VerifyReport { ok, detail }` after a protocol-appropriate probe;
    /// see `HttpOneShotAdapter` for the reference implementation.
    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError>;

    /// Reattach to an EXISTING upload session, restoring protocol state from
    /// `token` — the journaled `probe` event's `session_token` (ADR-002: the
    /// journal is the only truth). **Required, no default** — the taxonomy
    /// promises `Action::Reattach` for `SessionLost` (issue #12); a silent
    /// default would let an adapter promise recovery it never performs (the
    /// verify-is-required lesson, issue #1).
    ///
    /// Must never CREATE a session — re-probing would orphan the live one
    /// (an S3 re-probe opens a new multipart upload). Adapters whose probes
    /// never produce a `session_token` (one-shot endpoints) implement this
    /// as an explicit no-op; the manager only calls it when a journaled
    /// probe event actually carries a token.
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError>;
}

/// Registry — the plugin point community contributors use to add protocols
/// (e.g. a new `ium-adapter-tus` crate) without touching the core.
#[derive(Default, Clone)]
pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn TransferAdapter>>,
}

impl AdapterRegistry {
    /// An empty registry — register adapters before handing it to a manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an adapter; its [`TransferAdapter::name`] becomes its registry key.
    pub fn register(&mut self, adapter: Arc<dyn TransferAdapter>) {
        self.adapters.push(adapter);
    }

    /// Resolve by registry key (`JobSpec.adapter`).
    pub fn get(&self, name: &str) -> Option<Arc<dyn TransferAdapter>> {
        self.adapters.iter().find(|a| a.name() == name).cloned()
    }

    /// All registered keys, in registration order (the UI's adapter select
    /// reflects this list — #16; the UI never hard-codes protocols).
    pub fn names(&self) -> Vec<&'static str> {
        self.adapters.iter().map(|a| a.name()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OneShot;

    #[async_trait]
    impl TransferAdapter for OneShot {
        fn name(&self) -> &'static str {
            "one_shot_post"
        }
        fn capabilities(&self) -> Capability {
            Capability::default() // no flags: physically cannot resume
        }
        async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
            Ok(ProbeReport {
                resumable: false,
                max_chunk_bytes: None,
                max_parallel: None,
                session_token: None,
            })
        }
        async fn upload_chunk(
            &self,
            _spec: &JobSpec,
            lease: &Lease,
            progress: &ProgressReporter,
        ) -> Result<ChunkConfirm, AdapterError> {
            let span = lease.end - lease.resume_from;
            progress.add(span);
            Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: span,
                adapter_token: None,
            })
        }
        // Required since issue #1: verify is not opt-in. This double talks to
        // no real endpoint, so it states that honestly instead of inheriting
        // a silent pass.
        async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
            Ok(VerifyReport {
                ok: true,
                detail: Some("test double: no real endpoint to verify against".into()),
            })
        }
        // Required since issue #12: reattach is not opt-in either. This
        // one-shot double has no session, so the no-op is the honest body.
        async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
            Ok(())
        }
    }

    fn spec() -> JobSpec {
        JobSpec {
            source_path: "/tmp/f".into(),
            source_size: 10,
            adapter: "one_shot_post".into(),
            endpoint: "https://x/post".into(),
        }
    }

    #[test]
    fn badge_reflects_capability_honesty() {
        assert_eq!(Capability::default().badge(), "one_shot");
        assert_eq!(Capability::new(Capability::RESUMABLE).badge(), "resumable");
        assert!(
            Capability::new(Capability::RESUMABLE | Capability::TAIL_RACE)
                .has(Capability::TAIL_RACE)
        );
    }

    #[tokio::test]
    async fn registry_resolves_and_mock_runs() {
        let mut reg = AdapterRegistry::new();
        reg.register(Arc::new(OneShot));
        assert_eq!(reg.names(), vec!["one_shot_post"]);

        let a = reg.get("one_shot_post").unwrap();
        let s = spec();
        let probe = a.probe(&s).await.unwrap();
        assert!(!probe.resumable);

        let lease = Lease {
            idx: 0,
            begin: 0,
            end: 10,
            resume_from: 0,
            attempt: 0,
        };
        let progress = ProgressReporter::new();
        let confirm = a.upload_chunk(&s, &lease, &progress).await.unwrap();
        assert_eq!(confirm.bytes_confirmed, 10);
        assert_eq!(progress.moved(), 10, "adapter must report wire bytes");

        let v = a.verify(&s).await.unwrap();
        assert!(v.ok);
    }
}
