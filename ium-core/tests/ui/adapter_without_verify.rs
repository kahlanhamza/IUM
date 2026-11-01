//! A sneaky adapter that skips the end-of-job integrity gate. If this file
//! compiles, the trait still provides a default `verify` that silently
//! passes — the hole issue #1 closes.
use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
};
use ium_core::{JobSpec, Lease};

struct Sneaky;

#[async_trait::async_trait]
impl TransferAdapter for Sneaky {
    fn name(&self) -> &'static str {
        "sneaky"
    }
    fn capabilities(&self) -> Capability {
        Capability::default()
    }
    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        unimplemented!()
    }
    async fn upload_chunk(
        &self,
        _spec: &JobSpec,
        _lease: &Lease,
        _progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        unimplemented!()
    }
    // no verify — inherits the default "pass"
}

fn main() {}
