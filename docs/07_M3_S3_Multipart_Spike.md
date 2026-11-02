# M3 Spike — S3 multipart against MinIO (design note + issue breakdown)

> Spike scope: DESIGN ONLY. Implementation lands per-issue with failing-test-first
> evidence, same as every other fix. This document is the reference the M3 issues
> cite; STATUS.md §4 owns the current pointer.

## 0. Why S3 first

The spec was written S3-aware from day one (`02_IUM_MVP_Spec_v1.md` §4.1:
`CHUNK_MIN = 5 MiB` — exactly S3's minimum non-final part size; `CHUNK_MAX = 256 MiB`
with the note "S3 max part = 5 GiB, but 256 MiB keeps parts lists sane";
`Chunk.adapter_token` doc literally names "S3 part number"). S3 multipart is the
strictest mainstream transport, so building it first stress-tests every abstraction
the engine already has — and it is the DoD transport for the >100 MiB kill-9 drill.

## 1. Protocol ↔ `TransferAdapter` mapping

| Trait surface | S3 multipart operation | Notes |
|---|---|---|
| `probe(spec)` | `CreateMultipartUpload` | `session_token = UploadId`; `resumable = true`; `max_chunk_bytes = Some(5 GiB)` |
| `upload_chunk(lease)` | `UploadPart` | `partNumber = lease.idx + 1` (S3 is 1-based, idx is stable across re-leases); body = `[resume_from, end)` streamed from disk; `ETag` → `adapter_token = "N:etag"`; `bytes_confirmed = full span or Err` (parts are atomic — see §3) |
| `verify(spec)` | `ListParts` → `CompleteMultipartUpload` → `HEAD` | finalize-if-needed + verify; see §4 and ADR-010 proposal |
| `AdapterError::Transient` | network blips, 5xx | retry with counted attempts |
| `AdapterError::Throttled` | 503 `SlowDown`, `Retry-After` | policy honors `retry_after_ms` |
| `AdapterError::ExpiredSession` | `NoSuchUpload` on `UploadPart` | policy: `Reauthenticate` |
| `AdapterError::SessionLost` | upload id vanished between calls | policy: `Reattach` |
| `AdapterError::Unresumable` | endpoint refuses multipart (probe fallback) | policy: `Fail` |

Capability declaration (honest badges, ADR-004): `RESUMABLE | DYNAMIC_CHUNKS`.
Resume is **part-granularity** (§3) — the badge must not overpromise byte-resume.

## 2. Planner fit — what already works, what's broken

**Already right:** `ChunkPool::plan(total_len, n, chunk_min)` guarantees every chunk
≥ `chunk_min` when the length allows (n is clamped to `total_len / chunk_min`), and
`CHUNK_MIN = 5 MiB` matches S3's minimum non-final part size. A single chunk < 5 MiB
is legal on S3 when it is the only/final part — the planner produces exactly that for
small files (n collapses to 1).

**Found broken (filed, not scope-crept):**
1. **`ProbeReport.max_chunk_bytes` is discovered but never consumed** — no code path
   feeds it to the planner (grep: zero uses in `manager.rs`). Contract gap → issue #8:
   adapter declares, planner obeys (cap: `n >= ceil(total / max)`).
2. **0-byte files cannot use multipart.** S3 rejects 0-byte parts; issue #5's pinned
   `plan(0) = [(0,0)]` lease must map to `PutObject` (empty body) + HEAD verify in the
   S3 adapter. One lease, one round-trip, honest Done — invariant preserved.
3. **Tail race is safe:** two concurrent `UploadPart` calls with the same `partNumber`
   both succeed; the server keeps the last write; `CompleteMultipartUpload` assembles
   from the final `ListParts` state. No engine change needed.

## 3. Resume semantics — parts are atomic

S3 has no byte-level resume *within* a part: `UploadPart` either completes (ETag) or
the retry re-sends the whole part. Consequences the design accepts:

* `bytes_confirmed` is 0 or the full span — the partial-confirm path (issue #2
  lineage) will simply never see a mid-part confirm for S3. Correct, not a regression.
* Eviction/stall of an in-flight part costs the whole part re-send (≤ 256 MiB by
  `CHUNK_MAX`). Acceptable; noted for the speed inspector (spec §8) later.
* **Cross-restart rebuild (the kill-9 drill):** after a crash, completed parts are
  still recorded server-side. The adapter recovers by `ListParts` (partNumber →
  size + ETag) and confirms only those bytes in the journal — zero re-upload of
  completed parts. The drill asserts this via the wire-byte counter: bytes moved
  after restart < file size, exactly the completed-parts delta.

## 4. Finalize question — who calls `CompleteMultipartUpload` (ADR-010 proposal)

An uncompleted multipart upload has no object to verify. Options considered:
(a) complete inside the last `upload_chunk` — racy with tail race and re-rentals;
(b) new manager-driven finalize trait hook — trait churn before it's proven needed.

**Decision (proposed ADR-010): `verify()` = finalize-if-needed + verify** for S3:
`ListParts` → reconcile (every planned part present, Σ sizes == `source_size`) →
`CompleteMultipartUpload` → `HEAD` object (`Content-Length == source_size`) →
`VerifyReport{ok}`. Honest framing: for S3, verify is the **commit gate**, not a
read-only check — `Done` is declared only after the object actually exists.

* Idempotency: crash-after-complete → `CompleteMultipartUpload` returns `NoSuchUpload`
  → treat as already-completed → proceed to `HEAD`.
* Orphans (crash-before-complete): CI sets a MinIO lifecycle rule
  (`AbortIncompleteMultipartUpload`, 1 day). A manager→adapter abort hook is a
  deliberate non-goal for now (YAGNI; revisit with TUS).

## 5. Signing — hand-rolled SigV4, not the AWS SDK

`aws-sdk-s3` pulls the AWS common runtime (large dep tree) for ~6 operations we need.
Decision: **hand-roll SigV4** in `ium-core/src/sigv4.rs` on `hmac` + `sha2` + `hex`
(~150 lines, no network surface): canonical request → string-to-sign → derived key →
`Authorization` header. Provable offline against the official AWS SigV4 test vectors
(official test suite vectors ported as unit tests).

* Streaming bodies sign as `x-amz-content-sha256: UNSIGNED-PAYLOAD` (avoids
  chunk-signature machinery on the hot path; TLS still protects the wire; MinIO
  accepts it).
* Small XML bodies (`Create`/`Complete`) use the **real** SHA-256 — cheap, stricter.

## 6. S3 test server in CI — ~~one binary, all three OSes~~ (amended 2026-11-02)

> **AMENDMENT (2026-11-02, issue #7 rehearsal): MinIO is dead.** While preparing the #7 test
> environment, `dl.min.io` answered **410 Gone** for every community path. Timeline (verified
> against public sources the same day): console stripped May 2025 → binaries + Docker images
> cut Oct 2025 → "maintenance mode" Dec 2025 → "no longer maintained" Feb 2026 → **repo
> archived Apr 25, 2026**. The §6 premise "MinIO ships a single static binary per OS" no
> longer holds. Successor candidates for the #9 CI harness: **Garage** (Rust, single static
> binary, full multipart — the #7 live rehearsal ran against Garage v2.1.0 with zero
> adapter changes) and **SeaweedFS** (Go, Apache-2.0, `weed server -s3`). Legacy MinIO
> binaries are rejected: unmaintained supply-chain surface. RustFS rejected for now: alpha,
> self-declared not production-ready. The **adapter and its tests name no server product**
> — plain S3 over `IUM_S3_ENDPOINT` — so the #7 acceptance criteria are unaffected.

Original §6 (server name now historical): one pinned binary per runner, launched as a
process, waited on its health endpoint, env exported. The env-gated pattern below stands
unchanged — only the binary name/pinning mechanics move to #9's design note:

Roadmap M3 said "services container"; amended here: **`services:` containers are
Linux-only** on GitHub Actions, which would make the S3 gate Linux-only. MinIO ships
a single static binary per OS (linux/darwin/windows) — download a **pinned version**
per runner, launch as a process, wait on `/minio/health/ready`, export env:

```
IUM_S3_ENDPOINT=http://127.0.0.1:9000
IUM_S3_ACCESS_KEY=minioadmin   # throwaway CI/local creds only
IUM_S3_SECRET_KEY=minioadmin
IUM_S3_BUCKET=ium-test
```

Env-gated tests (pattern proven by the fuzz gating): when `IUM_S3_ENDPOINT` is unset,
S3 tests print `skipped` and pass — local `cargo test` needs no MinIO; CI and nightly
always export the env. `scripts/minio-test.sh` encapsulates setup for local use.
Nightly runs the >100 MiB + kill-9 drill (release profile).

## 7. Issue breakdown (milestone M3: Real transports)

| # | Title | Key acceptance criteria |
|---|---|---|
| #6 | `sigv4`: AWS Signature V4 signer (pure, offline) | canonical request / string-to-sign / derived key / Authorization header; UNSIGNED-PAYLOAD + real-SHA256 modes; official AWS test vectors as unit tests; deps limited to `hmac`, `sha2`, `hex` |
| #7 | `adapter_s3`: multipart adapter core | mapping per §1; `partNumber = idx+1` + `adapter_token = "N:etag"`; verify per ADR-010 (ListParts → Complete → HEAD); `NoSuchUpload` idempotency; 0-byte PutObject path; error mapping per §1; env-gated tests vs MinIO. Blocked by #6 |
| #8 | planner/adapter contract: consume `ProbeReport.max_chunk_bytes` | planner raises `n >= ceil(total/max)` when the cap binds; unit tests; ADR: adapter declares, planner obeys. Independent of #7 |
| #9 | MinIO CI harness (3 OS) + kill-9 resume drill + >100 MiB nightly e2e | pinned binary per OS + health wait + env export; env-gated locally; drill proves zero re-upload of completed parts via wire-byte counter; honest Done only after adapter verify. Blocked by #7 |

Order: #6 → #7 → #9 (with #8 parallelizable anywhere). TUS 1.0.0 adapter is the next
M3 tranche after S3 lands — it will reuse the ADR-010 finalize pattern and the CI
harness shape. Roadmap M3 DoD (kill-9, zero corruption, honest Done) maps to #9.

## 8. Risks / open questions

* **SigV4 edge cases** (virtual-host vs path-style, region scoping) — mitigated by
  official test vectors + path-style against MinIO; real AWS endpoint stays out of
  scope for tests (no credentials in CI).
* **`ListParts` pagination** (>1000 parts impossible under `CHUNK_MAX=256 MiB` × 1000
  parts = 256 GiB > we'd need >256 GiB files to paginate; still: implement the
  `part-number-marker` loop in #7 for correctness).
* **Windows binary launch semantics** (exe download + process spawn) — #9 must prove
  it on the Windows runner, not assume it.
* **Clock skew** in SigV4 (x-amz-date) — MinIO in CI runs on the runner clock; drift
  is a non-issue in-scope, noted for real deployments.
