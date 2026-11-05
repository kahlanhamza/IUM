//! The `fec` pseudo-adapter (issue #52): RaptorQ parity over ANY resumable
//! lane, behind the [`TransferAdapter`] trait — the client half of relay mode.
//!
//! ## Shape (design note §4, D1–D7 + the W13 addendum)
//!
//! * **Group grid derives from `source_size` alone** (pure [`crate::fec`]
//!   geometry) — resplit-stable by construction; the chunk pool's piece grid
//!   (declared cap = floor = `G`, the resplit freeze) never invalidates it.
//! * **The wrapped lane sees ordinary specs**: each group's framed symbol
//!   stream ([`crate::fec::symbolize_group`]) is written to a STAGING file
//!   (`<source>.ium-fec/gNNNNNN.sym`, deterministic, reused when present) and
//!   uploaded through the inner lane as an ordinary resumable job.
//! * **Per-group LANE INSTANCES, not per-group endpoints** (wire fact W13,
//!   rehearsed live: tusd 2.10.1 answers creation POSTs only at exactly
//!   `/files/` — any other path 405s, so paths cannot separate groups). The
//!   adapter holds a LANE FACTORY and builds one isolated inner instance per
//!   group (each tus instance carries its own session map — no key
//!   collision); server-side uniqueness comes from the server's own ids
//!   (tusd Locations). For stateless doubles the shared-`new` constructor
//!   clones one instance for every group.
//! * **Eager sessions at probe** (the #50 concat-lane shape): every group
//!   session exists before byte one; the probe event's `session_token` JSON
//!   carries the `fec_plan` geometry + every inner token — ADR-002's single
//!   journaled truth for reattach ([`#12`](crate::manager) replays it).
//! * **Spec rule #1 confirms**: source bytes of WIRE-COMPLETE groups ∩ the
//!   lease span, never client optimism; a partially delivered group confirms
//!   ZERO (its missing packets may be source symbols). Wire bytes — the
//!   (K+R)/K parity included — flow through the [`ProgressReporter`] as
//!   eviction telemetry. Per-group symbol receipts ride the chunk confirms'
//!   `adapter_token`s (journaled on chunk rows).
//! * **Verify** = AND over group HEADs (read-only), with the relay honesty in
//!   the detail: destination byte-exactness is the relay's decode job.
//!   **Finalize** removes staging (verify needs only the sessions).
//!
//! The 0-byte file stays a first-class citizen (issue #5): one empty group →
//! one 6-byte header session, driven and verified like any other.
//!
//! FEC is OPT-IN by construction (design note D7): the host registers
//! `FecAdapter` explicitly; no env vars, no JobSpec changes, zero manager
//! diff — plain lanes are byte-identical to pre-#52.

use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::fec::{FecConfig, FecGeometry};
use crate::JobSpec;

/// The staging directory for a fec job: `<source_path>.ium-fec/`.
pub fn staging_dir(spec: &JobSpec) -> PathBuf {
    PathBuf::from(format!("{}.ium-fec", spec.source_path))
}

/// The staging stream file for group `g`.
pub fn staging_path(spec: &JobSpec, g: u32) -> PathBuf {
    staging_dir(spec).join(format!("g{g:06}.sym"))
}

/// A lane factory: one ISOLATED inner-lane instance per group session
/// (wire fact W13 — servers that create only at a fixed path need instance
/// isolation, because adapters key sessions by endpoint).
pub type LaneFactory = Arc<dyn Fn() -> Arc<dyn TransferAdapter> + Send + Sync>;

/// One group's live lane: its own isolated inner instance (own session map).
/// The journaled session token lives in the probe event (ADR-002) — the
/// reattach path re-parses it from there, so the lane keeps no copy.
struct GroupLane {
    inner: Arc<dyn TransferAdapter>,
}

/// The FEC pseudo-adapter. Construct it with a wrapped lane (or a lane
/// factory) and a [`FecConfig`]; register it under its name (`"fec"`) and
/// set `spec.adapter` to match.
pub struct FecAdapter {
    factory: LaneFactory,
    /// The wrapped lane family's registry name, captured once at
    /// construction (one factory call, no per-call throwaway instances).
    lane_name: &'static str,
    config: FecConfig,
    /// Per-group server-acked WIRE bytes (telemetry state; re-derived from
    /// the server after reattach — cleared there, the inner lane's realign
    /// owns the truth).
    wire_confirmed: Mutex<HashMap<u32, u64>>,
    /// The per-group lanes, created at probe (eager sessions) and rebuilt at
    /// reattach (fresh process, token replay — #12 machinery composed).
    lanes: Mutex<Vec<GroupLane>>,
}

impl FecAdapter {
    /// Wrap ONE inner lane with RaptorQ parity per `config` — every group
    /// shares the instance. Right for stateless lanes and test doubles;
    /// endpoint-keyed servers (tusd, W13) want [`FecAdapter::with_factory`].
    pub fn new(inner: Arc<dyn TransferAdapter>, config: FecConfig) -> Self {
        FecAdapter::with_factory(
            Arc::new(move || inner.clone())
                as Arc<dyn Fn() -> Arc<dyn TransferAdapter> + Send + Sync>,
            config,
        )
    }

    /// Wrap a LANE FACTORY: one isolated inner instance per group (W13).
    /// For tus: `with_factory(Arc::new(|| Arc::new(TusAdapter::new())), cfg)`.
    pub fn with_factory(factory: LaneFactory, config: FecConfig) -> Self {
        let lane_name = factory().name();
        FecAdapter {
            factory,
            lane_name,
            config,
            wire_confirmed: Mutex::new(HashMap::new()),
            lanes: Mutex::new(Vec::new()),
        }
    }

    /// The derived spec the inner lane sees for group `g`: the staging stream
    /// as the source, the group's wire length as the size, the job's endpoint
    /// (per-group isolation comes from the lane instances, W13).
    fn group_spec(&self, spec: &JobSpec, geo: &FecGeometry, g: u32) -> JobSpec {
        JobSpec {
            source_path: staging_path(spec, g).to_string_lossy().into_owned(),
            source_size: geo.wire_len(g),
            adapter: self.lane_name.to_string(),
            endpoint: spec.endpoint.clone(),
        }
    }

    /// One fresh inner instance for a group lane.
    fn factory(&self) -> Arc<dyn TransferAdapter> {
        (self.factory)()
    }

    /// Write any missing staging streams (deterministic re-derivation — a
    /// resumed job reuses what is there; a finalize-then-repair re-drives
    /// rewrite from the source). One seek-read pass over the source.
    fn ensure_staging(&self, spec: &JobSpec, geo: &FecGeometry) -> std::io::Result<()> {
        std::fs::create_dir_all(staging_dir(spec))?;
        let mut file = std::fs::File::open(&spec.source_path)?;
        for (g, (start, end)) in geo.group_ranges().into_iter().enumerate() {
            let path = staging_path(spec, g as u32);
            if path.exists() {
                continue;
            }
            let mut buf = vec![0u8; (end - start) as usize];
            file.seek(std::io::SeekFrom::Start(start))?;
            file.read_exact(&mut buf)?;
            let stream = crate::fec::symbolize_group(geo, g as u32, &buf);
            std::fs::write(&path, &stream)?;
        }
        Ok(())
    }
}

#[async_trait]
impl TransferAdapter for FecAdapter {
    fn name(&self) -> &'static str {
        "fec"
    }

    /// Badges pass through from the wrapped lane (honesty is the lane's).
    fn capabilities(&self) -> Capability {
        // A const capture would need a second stored field; the factory call
        // is stateless for every real lane (tus: an empty map).
        self.factory().capabilities()
    }

    /// Derive geometry, write staging, create EVERY group session eagerly
    /// (one isolated lane instance per group), journal the `fec_plan` +
    /// tokens in the probe event (ADR-002 — the concat-lane shape, #50).
    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        let geo = FecGeometry::derive(&self.config, spec.source_size);
        self.ensure_staging(spec, &geo)
            .map_err(|e| AdapterError::Transient(format!("fec staging: {e}")))?;
        self.wire_confirmed.lock().unwrap().clear();

        let mut lanes = Vec::with_capacity(geo.group_count() as usize);
        let mut tokens = Vec::with_capacity(geo.group_count() as usize);
        let mut resumable = true;
        for g in 0..geo.group_count() {
            let inner = self.factory();
            let spec_g = self.group_spec(spec, &geo, g);
            let rep = inner.probe(&spec_g).await?;
            resumable &= rep.resumable;
            let token = rep.session_token;
            tokens.push(json!(token.clone().unwrap_or_default()));
            lanes.push(GroupLane { inner });
        }
        *self.lanes.lock().unwrap() = lanes;

        let mut plan = geo.journal_payload();
        plan["groups"] = json!(tokens);
        Ok(ProbeReport {
            uniform_block: None,
            resumable,
            max_chunk_bytes: Some(geo.group_bytes),
            min_chunk_bytes: Some(geo.group_bytes),
            max_parallel: None,
            session_token: Some(plan.to_string()),
            concatenation: false,
        })
    }

    /// Drive every group intersecting `[lease.resume_from, lease.end)` to
    /// wire-completion through its own lane instance; confirm span-honest
    /// source bytes of wire-complete groups only (D5).
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let geo = FecGeometry::derive(&self.config, spec.source_size);
        self.ensure_staging(spec, &geo)
            .map_err(|e| AdapterError::Transient(format!("fec staging: {e}")))?;

        let mut confirmed = 0u64;
        let mut receipts: Vec<serde_json::Value> = Vec::new();
        for g in 0..geo.group_count() {
            let (gs, ge) = (
                g as u64 * geo.group_bytes,
                g as u64 * geo.group_bytes + geo.group_true_len(g),
            );
            // The empty file's single [0,0) group still runs (issue #5's
            // round-trip invariant): intersect = overlap for non-empty
            // geometry, group 0 always for the empty one.
            let intersects = if geo.source_size == 0 {
                g == 0
            } else {
                gs.max(lease.resume_from) < ge.min(lease.end)
            };
            if !intersects {
                continue;
            }
            let wire_total = geo.wire_len(g);
            let done = *self.wire_confirmed.lock().unwrap().get(&g).unwrap_or(&0);
            if done < wire_total {
                let lane = self
                    .lanes
                    .lock()
                    .unwrap()
                    .get(g as usize)
                    .map(|l| l.inner.clone())
                    .ok_or_else(|| {
                        AdapterError::SessionLost(format!(
                            "fec group {g} has no session — drive before probe/reattach"
                        ))
                    })?;
                let lease_w = Lease {
                    idx: g,
                    begin: 0,
                    end: wire_total,
                    resume_from: done.min(wire_total),
                    attempt: lease.attempt,
                };
                let spec_g = self.group_spec(spec, &geo, g);
                let c = lane.upload_chunk(&spec_g, &lease_w, progress).await?;
                let now = (done + c.bytes_confirmed).min(wire_total);
                self.wire_confirmed.lock().unwrap().insert(g, now);
                receipts.push(json!({"g": g, "tok": c.adapter_token}));
            }
            let now = *self.wire_confirmed.lock().unwrap().get(&g).unwrap_or(&0);
            if now >= wire_total {
                let lo = lease.resume_from.max(gs);
                let hi = lease.end.min(ge);
                if hi > lo {
                    confirmed += hi - lo;
                }
            }
        }
        let adapter_token = if receipts.is_empty() {
            None
        } else {
            Some(json!({"fec": receipts}).to_string())
        };
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: confirmed,
            adapter_token,
        })
    }

    /// AND over group HEADs; the detail states the relay honesty (D6).
    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let geo = FecGeometry::derive(&self.config, spec.source_size);
        // Clone the lane handles out of the guard — a MutexGuard must never
        // cross an await (Send futures).
        let lanes: Vec<Arc<dyn TransferAdapter>> = {
            let guard = self.lanes.lock().unwrap();
            if guard.len() != geo.group_count() as usize {
                return Ok(VerifyReport {
                    ok: false,
                    detail: Some(format!(
                        "fec: {} of {} group sessions exist (probe/reattach first) — the relay's decode job cannot be gated on an incomplete lane",
                        guard.len(),
                        geo.group_count()
                    )),
                });
            }
            guard.iter().map(|l| l.inner.clone()).collect()
        };
        let mut failed: Vec<String> = Vec::new();
        for (g, inner) in lanes.iter().enumerate() {
            let spec_g = self.group_spec(spec, &geo, g as u32);
            match inner.verify(&spec_g).await {
                Ok(rep) if rep.ok => {}
                Ok(rep) => failed.push(format!(
                    "g{g}: {}",
                    rep.detail.unwrap_or_else(|| "unverified".into())
                )),
                Err(e) => failed.push(format!("g{g}: {e}")),
            }
        }
        let detail = format!(
            "fec: {}/{} group sessions server-complete (HEAD per group, wire overhead {:.4}); destination byte-exactness (K-of-(K+R) decode) is the relay's job{}",
            geo.group_count() - failed.len() as u32,
            geo.group_count(),
            geo.overhead_ratio(),
            if failed.is_empty() {
                String::new()
            } else {
                format!("; failed: {}", failed.join(", "))
            }
        );
        Ok(VerifyReport {
            ok: failed.is_empty(),
            detail: Some(detail),
        })
    }

    /// Replay every group token into its lane instance (the #12 machinery,
    /// composed). A fresh process has no lanes yet — the factory builds them
    /// from the token's group count; wire progress re-derives from the
    /// server afterwards.
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        let v: serde_json::Value = serde_json::from_str(token)
            .map_err(|e| AdapterError::Unresumable(format!("fec token: {e}")))?;
        let groups = v["groups"]
            .as_array()
            .ok_or_else(|| AdapterError::Unresumable("fec token missing groups".into()))?;
        self.wire_confirmed.lock().unwrap().clear();
        let mut lanes = Vec::with_capacity(groups.len());
        for tok in groups.iter() {
            let tok = tok.as_str().unwrap_or("");
            let inner = self.factory();
            if !tok.is_empty() {
                inner.reattach(endpoint, tok).await?;
            }
            lanes.push(GroupLane { inner });
        }
        *self.lanes.lock().unwrap() = lanes;
        Ok(())
    }

    /// Remove the staging directory (best-effort) — verify needs only the
    /// sessions, so the read-only verify keeps its semantics (D6).
    async fn finalize(&self, spec: &JobSpec) -> Result<Option<String>, AdapterError> {
        let dir = staging_dir(spec);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!("fec staging cleanup failed (best-effort): {e}");
            }
        }
        Ok(None)
    }
}

/// Re-export for the drill: parse a staging stream without reaching into the
/// crate internals beyond the public `fec` API.
pub fn parse_staging(
    spec: &JobSpec,
    g: u32,
) -> std::io::Result<(u64, u16, Vec<raptorq::EncodingPacket>)> {
    let bytes = std::fs::read(staging_path(spec, g))?;
    crate::fec::parse_stream(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
