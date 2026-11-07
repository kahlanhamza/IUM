//! Hot lane registration (issue #81): saving credentials in the shell must
//! re-register a lane WITHOUT an app restart. The owner's bar: auth "should
//! not be painful, it should be so easy" — "takes effect the next time IUM
//! starts" is the pain this kills.
//!
//! The supervisor hands the manager out as `Arc<JobManager>`
//! (`Supervisor::manager()`); there is no `&mut` path back from a Tauri
//! command. So the upsert seam must be usable through the shared handle —
//! `JobManager::replace_adapter(&self, name, adapter)`.
//!
//! Contract:
//! * upsert by adapter name: replacing a KNOWN name swaps the config in
//!   place (exactly one registry entry remains);
//! * inserting an UNKNOWN name adds the lane — the first credential entry
//!   for a lane boot never registered (no env credentials at boot);
//! * in-flight jobs are never disturbed: the engine resolves the adapter
//!   Arc at `create_job`, so a swap only affects jobs created after it —
//!   the method works on `&self` precisely because nothing retroactive is
//!   possible.

use std::sync::Arc;

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::{JobManager, JobSpec, Lease};

struct Mock {
    name: &'static str,
    resumable: bool,
}

impl Mock {
    fn arc(name: &'static str, resumable: bool) -> Arc<Self> {
        Arc::new(Mock { name, resumable })
    }
}

#[async_trait::async_trait]
impl TransferAdapter for Mock {
    fn name(&self) -> &'static str {
        self.name
    }
    fn capabilities(&self) -> Capability {
        if self.resumable {
            Capability::new(Capability::RESUMABLE)
        } else {
            Capability::default()
        }
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        Ok(ProbeReport {
            uniform_block: None,
            resumable: self.resumable,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: None,
            session_token: None,
            concatenation: false,
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
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("test double: no real endpoint to verify against".into()),
        })
    }
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }
}

#[test]
fn registry_upsert_swaps_a_known_name_in_place() {
    let mut reg = ium_core::adapter::AdapterRegistry::new();
    reg.register(Mock::arc("mock_lane", false));
    assert_eq!(reg.names(), vec!["mock_lane"]);

    // Replace: true = a known name was swapped; still exactly one entry.
    assert!(reg.replace(Mock::arc("mock_lane", true)));
    assert_eq!(reg.names(), vec!["mock_lane"]);
    let a = reg.get("mock_lane").unwrap();
    assert!(a.capabilities().has(Capability::RESUMABLE));
}

#[test]
fn registry_upsert_inserts_an_unknown_name() {
    let mut reg = ium_core::adapter::AdapterRegistry::new();
    reg.register(Mock::arc("mock_lane", false));

    // First credential entry for a lane boot never registered: false =
    // not a replacement, a NEW lane joined the registry (insertion order
    // preserved for the UI's adapter select).
    assert!(!reg.replace(Mock::arc("late_lane", true)));
    assert_eq!(reg.names(), vec!["mock_lane", "late_lane"]);
}

#[test]
fn manager_replace_adapter_works_through_the_shared_arc() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("hot.db");
    let journal = ium_core::SessionJournal::open(&db).unwrap();
    let mut manager = JobManager::with_journal(journal).unwrap();
    // Boot path: register while the manager is still unshared (&mut).
    manager.register_adapter(Mock::arc("mock_lane", false));
    let manager = Arc::new(manager); // the supervisor's handoff shape
                                     // The hot path: &self through an Arc — no `get_mut`, no restart.
    assert!(manager.replace_adapter(Mock::arc("mock_lane", true)));
    assert_eq!(manager.adapter_names(), vec!["mock_lane"]);
    // The late insert path through the same handle.
    assert!(!manager.replace_adapter(Mock::arc("late_lane", true)));
    assert_eq!(manager.adapter_names(), vec!["mock_lane", "late_lane"]);
    // Removing credentials removes the lane, honestly reported.
    assert!(manager.remove_adapter("late_lane"));
    assert!(!manager.remove_adapter("late_lane"));
    assert_eq!(manager.adapter_names(), vec!["mock_lane"]);
}
