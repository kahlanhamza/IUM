//! Transfer adapters — one module per protocol family, behind capability
//! flags (pattern #7: FDM's `AbstractDownloadModule` registry shape,
//! rebuilt for uploads).

use async_trait::async_trait;
use std::sync::Arc;

use crate::chunk::Lease;
use crate::JobSpec;

/// Bitflags a protocol adapter declares honestly — the UI turns these into
/// the Resumable ✅ / One-shot ⚠️ badges (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capability(u8);

impl Capability {
    pub const RESUMABLE: u8 = 1 << 0;
    pub const DYNAMIC_CHUNKS: u8 = 1 << 1;
    pub const TAIL_RACE: u8 = 1 << 2;

    pub const fn new(bits: u8) -> Self {
        Capability(bits)
    }
    pub const fn has(self, bit: u8) -> bool {
        self.0 & bit == bit
    }
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
    /// Opaque session descriptor to carry across confirms.
    pub session_token: Option<String>,
}

/// A chunk that completed with a server acknowledgment.
#[derive(Debug, Clone)]
pub struct ChunkConfirm {
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
    pub ok: bool,
    pub detail: Option<String>,
}

/// Errors adapters return, already classified for the retry policy.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("throttled: {message}")]
    Throttled {
        retry_after_ms: Option<u64>,
        message: String,
    },
    #[error("expired session: {0}")]
    ExpiredSession(String),
    #[error("session lost: {0}")]
    SessionLost(String),
    #[error("unresumable: {0}")]
    Unresumable(String),
    #[error("corruption: {0}")]
    Corruption(String),
    #[error("auth required: {0}")]
    AuthRequired(String),
    #[error("unknown: {0}")]
    Unknown(String),
}

impl AdapterError {
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
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
    ) -> Result<ChunkConfirm, AdapterError>;

    /// End-of-job integrity gate (pattern #10). Default: pass.
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: None,
        })
    }
}

/// Registry — the plugin point community contributors use to add protocols
/// (e.g. a new `ium-adapter-tus` crate) without touching the core.
#[derive(Default, Clone)]
pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn TransferAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, adapter: Arc<dyn TransferAdapter>) {
        self.adapters.push(adapter);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn TransferAdapter>> {
        self.adapters.iter().find(|a| a.name() == name).cloned()
    }

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
                session_token: None,
            })
        }
        async fn upload_chunk(
            &self,
            _spec: &JobSpec,
            lease: &Lease,
        ) -> Result<ChunkConfirm, AdapterError> {
            Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: lease.end - lease.begin,
                adapter_token: None,
            })
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
        let confirm = a.upload_chunk(&s, &lease).await.unwrap();
        assert_eq!(confirm.bytes_confirmed, 10);

        let v = a.verify(&s).await.unwrap();
        assert!(v.ok);
    }
}
