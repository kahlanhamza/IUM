# Issue #57 — design note: the S3-shaped link prober (multipart transactions)

Wire facts FIRST (scripts/wirefacts57/, both pinned servers, probed live
2026-11-05 — evidence.md): the probe shape the issue asks about is CHEAP
and leaves NO orphan, on both canonical servers. Every decision below
cites its wire fact.

## Wire facts (W-numbers, this run)

- **W0/W0b**: the anonymous `rclone serve s3` answers anonymous `GET /` 200;
  SIGNED service-level ops (ListBuckets on `/`, PutBucket on `/bkt`) break
  mid-transfer on gofakes3, while signed OBJECT ops ride clean. The prober
  and the engine speak object ops only; buckets pre-exist (W10, #63).
- **W0c** (the rehearsal's own curl lesson): `curl --aws-sigv4` takes ONE
  argument (`provider:region:service`) — keys ride `--user`. A second
  `aws:iam::…` string is parsed as a SECOND URL (curl error 3) and the
  request silently goes out anonymous.
- **W1**: UploadPart accepts a 64 KiB part into a LIVE multipart upload on
  BOTH servers (rclone 200 + real ETag; SeaweedFS 200 + real ETag). The 5
  MiB floor does NOT bind at UploadPart.
- **W2**: 4 concurrent part PUTs on ONE upload all succeed (200×4 on both) —
  parts are independent resources; the K-phase rides one upload, exactly
  the production shape (the adapter's fleet uploads concurrent parts of one
  upload).
- **W3**: ListParts mid-flight lists all 5 parts on both.
- **W4, the decisive divergence**: CompleteMultipartUpload with five small
  (64 KiB) parts and REAL ETags — **gofakes3 ACCEPTS it (200, ETag suffix
  `-5`)**; **SeaweedFS REJECTS it (400 EntityTooSmall)**. The floor is
  SERVER-DEPENDENT. Therefore the probe NEVER completes — it ABORTS.
  Completing a probe would materialize junk on lax servers and fail on
  strict ones. Abort is the only honest cleanup.
- **W5**: AbortMultipartUpload → 204 on both; ListParts after abort → 404
  NoSuchUpload on both; `GET /bkt?uploads` (ListMultipartUploads) works on
  both and is EMPTY after the abort — the drill's orphan proof is
  server-owned.
- **W6**: a full create → 64 KiB part → abort transaction costs ~19–25 ms
  on localhost. The probe's budget is the canary BYTES on the wire, not the
  transaction overhead.

## Design

**D1 — the trait channel (ADR-011: the adapter declares, the planner
obeys).** `TransferAdapter::probe_link(&self, spec, k, sample_hint_bps) ->
Result<Option<LinkProfile>, AdapterError>`, default `Ok(None)` — a lane
without a shaped prober is byte-identical to pre-#57 behavior (no journal
noise). `TusAdapter` overrides and delegates to the existing free
`prober::probe_link` (same signature, same bytes); `S3MultipartAdapter`
overrides with the new multipart leg; `RcloneBridgeAdapter` overrides by
delegating to its inner S3 adapter over `inner_spec` (the #63 delegation
shape) — the bridge lane gets the prober for free. The manager boundary
drops the `spec.adapter == "tus"` string check for the LINK probe and calls
the trait; the #51 path plan keeps its tus-wire gate (the path prober
speaks tus wire).

**D2 — the multipart probe transaction (W1/W2/W4/W5/W6).**
Phase 1: `CreateMultipartUpload` on a probe key (the job's object key with
a `.ium-probe` suffix — never collides with the real object), `UploadPart
partNumber=1` streaming the canary bytes from the SOURCE prefix (ADR-009:
the payload's own bytes, never synthetic zeros) → S1.
Phase K: `UploadPart partNumber=2..=K+1` CONCURRENTLY, part *i* streaming
`canary/K` bytes from offset `i·canary/K` — the aggregate phase moves the
same total bytes as phase 1 (the tus prober's wall-clock rule) → SK.
Cleanup: `AbortMultipartUpload` ALWAYS — on success and on every failure
path (W4: never complete; W5: abort is the no-orphan cleanup). The probe
upload id lives OUTSIDE `self.sessions` (a side instrument must never
disturb the lane's session map — the real upload's UploadId is untouched).
Process death mid-probe abandons the upload exactly like the tus prober's
abandoned sessions (documented in prober.rs since #41).

**D3 — classification is ONE pure fn, shared.** `prober::classify(s1, sk,
k, source_size) -> LinkProfile` — the same ≥2× → PerFlowCap/fleet=k,
<1.5× → BearerCap/fleet=1, between → Mixed/2 arithmetic and the same
piece rule (~20 s of S1, clamped [64 KiB, CHUNK_MIN] ∩ source_size), now
pinned by pure-fn contract tests and consumed by BOTH legs. The S3 lane's
5 MiB floor is NOT the prober's business: the measured piece is DATA, and
`apply_measured_piece` → `ChunkPool::new_capped_floor(declared_lane_floor)`
floors sub-5 MiB recommendations structurally (the engine default floor —
the S3 probe declares `min_chunk_bytes: None` precisely because CHUNK_MIN
is 5 MiB). Pinned by a contract test: an S3 recommendation of 64 KiB
re-plans the pool to ≥ 5 MiB chunks.

**D4 — the journal contract is unchanged.** Same `link_profile` event name,
same payload shape, same `link_profile_skipped` on failure, same
`measured_fleet()` journal read. Resume re-derives from the journal
(ADR-002) — nothing new to persist.

**D5 — no fabrication on failure.** A failed leg (create/part/abort error)
→ `link_profile_skipped` with the reason; the job proceeds on the planned
pool. A failed probe is a lost MEASUREMENT, never a lost job (#48's rule,
now lane-wide).

**D6 — opt-in unchanged.** Still `IUM_LINK_PROBE=1`. Default-off faces are
pinned (`no_link_probe_without_the_opt_in`).

## RED plan

1. Pure-fn classification contracts (`tests/prober_s3.rs`): the boundaries
   (2.0× inclusive PerFlowCap; 1.5× exclusive → Mixed; 1.0× BearerCap),
   piece clamps, source-size intersection.
2. Manager contracts (hermetic, a scripted adapter registered as
   `s3_multipart`): journal-once + `replan_measured` + the 5 MiB floor
   interaction; Err → `link_profile_skipped` + Done on the original plan;
   Ok(None) → zero journal noise.
3. Env-gated live drill (`tests/s3_probe_drill.rs`, `IUM_S3_PROBE_DRILL=1`):
   rclone serve s3 behind the #55 throttle proxy — per-conn-rate leg →
   PerFlowCap; total-rate leg → BearerCap; the orphan proof (bucket's
   multipart list empty after the probe) — the discriminating
   shared-bearer-vs-per-flow shape, server-owned.

The RED compiles-fails on the clean tree: `probe_link` is not yet a trait
method (E0599 on the scripted adapter), `classify` does not exist.
