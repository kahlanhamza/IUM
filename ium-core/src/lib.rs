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
//! * [`retry::RetryPolicy`] — per-error-class retry decisions
//!
//! Design rules (see docs/02_IUM_MVP_Spec_v1.md):
//! 1. bytes_confirmed is only what the SERVER acknowledged — never client optimism.
//! 2. Every state transition is journaled before it executes.
//! 3. The engine crate must not depend on any UI framework.

pub mod adapter;
pub mod chunk;
pub mod ids;
pub mod journal;
pub mod manager;
pub mod retry;
pub mod state;

pub use adapter::{AdapterRegistry, Capability, ChunkConfirm, ProbeReport, TransferAdapter};
pub use chunk::{Chunk, ChunkPool, ChunkState, Lease};
pub use ids::new_id;
pub use journal::{ChunkRow, JobRecord, SessionJournal};
pub use manager::{JobManager, JobSnapshot};
pub use state::JobState;

/// Crate version, surfaced by the desktop shell's status command.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One job's immutable specification, as submitted by the UI/CLI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobSpec {
    pub source_path: String,
    pub source_size: u64,
    /// Adapter registry key, e.g. "s3_multipart", "tus", "http_range".
    pub adapter: String,
    /// Normalized endpoint (bucket URL, tus endpoint, DAV root...).
    pub endpoint: String,
}

/// Reason an engine operation failed.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("adapter '{0}' is not registered")]
    UnknownAdapter(String),
    #[error("job {0} not found")]
    JobNotFound(String),
    #[error("illegal state transition: {0}")]
    IllegalTransition(#[from] state::StateError),
    #[error("journal error: {0}")]
    Journal(#[from] journal::JournalError),
    #[error("adapter error: {0}")]
    Adapter(String),
    #[error("job {0} is in state {1}; operation requires {2}")]
    WrongState(String, String, &'static str),
}

pub type Result<T> = std::result::Result<T, EngineError>;
