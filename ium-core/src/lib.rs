//! ium-core — the IUM transfer engine.
//!
//! Pure Rust, UI-agnostic, Tauri-agnostic. Everything the desktop shell, the
//! future CLI and the tests use lives behind this crate's facade:
//!
//! * [`chunk::ChunkPool`] — chunk algebra: split, lease, evict, resplit, tail race
//! * [`state::JobState`] — supervisor state machine (journaled transitions)
//! * [`journal::SessionJournal`] — SQLite (WAL) crash-safe session store
//! * [`adapter::TransferAdapter`] — per-protocol modules behind a capability flag set
//! * [`manager::JobManager`] — the orchestrator facade
//! * [`supervisor::Supervisor`] — spawn/pause/resume/cancel over one manager
//! * [`retry::RetryPolicy`] — per-error-class retry decisions
//! * [`eviction::EvictionPolicy`] — slow-connection eviction tuning (spec §4.2)
//! * [`fleet`] — the fleet decision algebra: measured seed (M8 #48) + live
//!   AIMD adaptation (M8 #49) — the engine answers the link's swings in BOTH
//!   directions
//!
//! ## Invariants (ADR-pinned, enforced by the test suite)
//!
//! 1. **The journal is the only truth** (ADR-002): state is read from the
//!    SQLite WAL journal, never from caller arguments; every transition is
//!    journaled BEFORE it executes; a fresh process rebuilds everything from
//!    the journal (crash recovery, `reattach` token replay — #12).
//! 2. **Verify-before-declare** (ADR-004, issue #1): `verify` is a REQUIRED
//!    trait method; nothing reports Done that the adapter did not verify
//!    (S3 additionally reconciles parts before Complete — ADR-010).
//! 3. **Adapter declares, planner obeys** (ADR-011/ADR-012): `ProbeReport`
//!    carries `max_chunk_bytes` (#8) and `max_parallel` (#13); the planner
//!    consumes both as hard constraints; no per-adapter special cases.
//! 4. **Counted retries** (spec §5): every failure path is counted and
//!    terminates in Done, Failed or Canceled — never an infinite retry.
//! 5. **Public API is frozen** (M5, issue #19): `#![deny(missing_docs)]`
//!    holds; `sigv4` and `ids` are private implementation details.
//!
//! Design rules (see docs/02_IUM_MVP_Spec_v1.md):
//! 1. bytes_confirmed is only what the SERVER acknowledged — never client optimism.
//! 2. Every state transition is journaled before it executes.
//! 3. The engine crate must not depend on any UI framework.

#![deny(missing_docs)]

pub mod adapter;
pub mod adapter_fec;
pub mod adapter_http;
pub mod adapter_s3;
pub mod adapter_tus;
pub mod chunk;
pub mod eviction;
pub mod fec;
pub mod fleet;
pub mod journal;
pub mod manager;
pub mod netpath;
pub mod prober;
pub mod retry;
pub mod state;
pub mod supervisor;

mod ids;
mod sigv4;

pub use adapter::{AdapterRegistry, Capability, ChunkConfirm, ProbeReport, TransferAdapter};
pub use adapter_fec::FecAdapter;
pub use adapter_http::HttpOneShotAdapter;
pub use adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
pub use adapter_tus::TusAdapter;
pub use chunk::{Chunk, ChunkPool, ChunkState, Lease};
pub use eviction::EvictionPolicy;
pub use fleet::{AdaptiveFleet, FleetChange, FleetReason};
pub use ids::new_id;
pub use journal::{ChunkRow, JobRecord, SessionJournal};
pub use manager::{JobManager, JobSnapshot};
pub use state::JobState;
pub use supervisor::Supervisor;

/// Crate version, surfaced by the desktop shell's status command.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One job's immutable specification, as submitted by the UI/CLI.
///
/// The engine treats this as the job's identity contract: `source_size` is
/// what the journal will believe until the job ends — an adapter re-checks
/// the file against it before every call and reports `Corruption` (never
/// silent loss) if the source changed underneath the job.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobSpec {
    /// Absolute path of the file to upload. Read-only for the engine; every
    /// byte the adapters stream comes from here.
    pub source_path: String,
    /// Size in bytes at job-creation time. Journal truth; adapters verify
    /// the on-disk size against it on every call.
    pub source_size: u64,
    /// Adapter registry key, e.g. "s3_multipart", "tus", "http_range".
    pub adapter: String,
    /// Normalized endpoint (bucket URL, tus endpoint, DAV root...).
    pub endpoint: String,
}

/// Reason an engine operation failed.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The requested adapter key is not in the registry.
    #[error("adapter '{0}' is not registered")]
    UnknownAdapter(String),
    /// No job with this id exists in the journal.
    #[error("job {0} not found")]
    JobNotFound(String),
    /// The journal's current state forbids this transition (state machine
    /// honesty — see `state::LEGAL`).
    #[error("illegal state transition: {0}")]
    IllegalTransition(#[from] state::StateError),
    /// The journal (SQLite WAL) failed a read or a loud-failure write.
    #[error("journal error: {0}")]
    Journal(#[from] journal::JournalError),
    /// An adapter reported an error while probing, uploading or verifying.
    #[error("adapter error: {0}")]
    Adapter(String),
    /// The operation is illegal in the job's CURRENT journal state.
    #[error("job {0} is in state {1}; operation requires {2}")]
    WrongState(String, String, &'static str),
}

/// Engine result alias: everything fails loudly through [`EngineError`].
pub type Result<T> = std::result::Result<T, EngineError>;
