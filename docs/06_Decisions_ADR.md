# IUM — Architecture Decision Records

Lightweight ADR log: one entry per decision, status, and the consequence we accept.
Ratified retroactively where marked.

## ADR-001 — Language & runtime: Rust + tokio, reqwest with rustls ✅ ratified (Sprint 0)
Crash-safety, streaming bodies, no OpenSSL cross-platform pain. Consequence: slower iteration
than GC languages; accepted because correctness is the product.

## ADR-002 — Journal: SQLite (WAL) via rusqlite, journal is the single source of truth ✅ ratified
Versioned migrations via `PRAGMA user_version`; job+chunk inserts transactional; every write
checks rows-affected and fails loudly; every state transition is journaled **before** it
executes; on boot the manager rebuilds reality from the journal. Consequence: all state reads
go through the journal — caller arguments are never trusted.

## ADR-003 — Chunk pool: dynamic segments with leases ✅ ratified
Files split into dynamic chunks; workers lease chunks; a partial confirm returns the chunk to
the rentable pool keeping its confirmed offset. Consequence: the premature-Done bug class is
structurally excluded, plus a `finish()` consistency gate as belt-and-suspenders.

## ADR-004 — Verify-before-declare ✅ ratified (hardening open, see Roadmap M2)
A job may only reach `Done` after the adapter verifies the upload (HTTP adapter: HEAD
Content-Length). **Open gap:** `TransferAdapter::verify` currently has a default "pass" impl —
M2 makes it a required method so no future adapter can inherit silence.

## ADR-005 — Slow-connection eviction: throughput watch + strikes ✅ ratified (2026-11-02)
Per-lease sliding-window throughput; a worker below `floor_ratio × median(pool)` for
`strikes_to_evict` consecutive windows is evicted (aria2's pattern, rebuilt for uploads, spec
§4.2). Pure decision logic, unit-tested without real clocks. Lone-worker and fully-idle pools
never evict (no thrash).

## ADR-006 — Engine first, UI frozen ✅ ratified (team directive); ✅ AMENDED 2026-11-02 (issue #14)
`ium-app` (Tauri 2 + React) does not join the workspace and is not compiled in CI until M2+M3.
Consequence: no demo UI until the engine is trustworthy — deliberate.

**Amendment (M2+M3 complete):** the freeze is lifted. `ium-app/src-tauri` joins the
workspace and CI gains a 3-OS `ium-app` job (frontend lane `npm ci && npm run build`
unconditional — tauri's `generate_context!` embeds `dist/`, so the build precedes cargo;
shell lane `cargo clippy -p ium-app --all-targets -- -D warnings` + `cargo test -p
ium-app` native on every runner OS). The LOCAL gate runs the frontend lane everywhere
and the shell lane only where the platform's libraries exist (WebKitGTK on Linux);
the sandbox has no root to install them, so its skip is LOUD in gate output and the
CI job owns the lane on every push — the same capability-gated honesty contract as
the env-gated S3/TUS live suites (skip locally is never silent; CI enforces).

## ADR-007 — Gates are non-bypassable ✅ ratified (2026-11-02)
No commit lands without `fmt --check` + `clippy -D warnings` + full test suite green locally.
Evidence: one ungated snapshot commit (`0c11211`) introduced a clippy hard error, a test that
could never pass, and fmt drift — all caught within one day once the gates were actually run.

## ADR-008 — STATUS.md is the single source of truth, pushed every task ✅ ratified (2026-11-02)
Team decision: sandbox recycles wipe context; GitHub holds the memory. Every task ends with
STATUS.md updated and pushed. A task that is not pushed does not exist.

## ADR-009 — Clean-room policy ✅ ratified
Vendor behavior is studied by observing printable strings and public documentation only
(`scripts/elf_strings_scan.py`, `scripts/idm_pe_scan.py`, `scripts/magic_carver.py`; evidence in
`rea-lab/out/`). No disassembly, no decompilation, no vendor code or assets in the repo.

## ADR-010 — For S3, verify() is the commit gate: finalize-if-needed + verify ✅ accepted (issue #7)
An uncompleted multipart upload has no object to verify, so the S3 adapter's `verify()` is
`ListParts` reconcile → `CompleteMultipartUpload` → `HEAD` (`Content-Length == source_size`).
Honest framing: verify is not read-only for S3 — `Done` is declared only after the object
actually exists. Idempotent (`NoSuchUpload` on ListParts **or** Complete = already finalized
→ go HEAD — both shapes observed live and pinned as tests); reconcile-before-Complete means
a wrong assembly is never completed. Orphans (crash-before-complete) are handled by a storage
lifecycle rule, not a manager hook (YAGNI until TUS proves the need). Accepted with issue #7's
evidence: 20/20 env-gated tests green live against a real S3 server, including
`verify-refuses-to-lie` (5 of 12 MiB present → ok:false, upload left incomplete).

## ADR-011 — Adapter declares, planner obeys ✅ accepted (issue #8, 2026-11-02)
`ProbeReport.max_chunk_bytes` constrains planning: the manager re-plans at the
probe boundary with `n >= ceil(total/cap)` whenever the declared ceiling binds
(`ChunkPool::plan_capped`). The chunk pool stays the single planning authority;
adapters contribute constraints, never plans. The cap is a HARD constraint — it
composes as `n_final = max(min(n, floor(total/chunk_min)), ceil(total/cap))`, so
when `cap < chunk_min` the cap wins and chunks dip below the policy floor: a
violated limit fails uploads, a violated floor only costs requests. One-shot
endpoints are never re-planned (chunking a non-resumable stream would imply a
capability the protocol does not have) — a binding cap on a one-shot adapter
fails loudly at the adapter instead. The re-plan is journaled (`replan_cap`
event + `replace_chunks`) and only runs while every chunk is still `Pending`
with zero confirmed bytes, verified from the journal (ADR-002). Accepted with
issue #8's evidence: RED (`got 2` on a 4 MiB cap) → GREEN, 100/100 gates
including not-binding-is-byte-identical, cap-vs-chunk_min conflict, boundary
and 0-byte invariants.

## ADR-012 — Parallelism is declared too ✅ accepted (issue #13, 2026-11-02)
`ProbeReport.max_parallel` extends the ADR-011 channel to CONCURRENCY: the
manager's drive round runs `min(pending, MAX_WORKERS, declared_max_parallel)`
workers, and TUS — one sequential-offset resource — declares `Some(1)`
(S3 parts are independent and declare `None`). The declaration is journaled
inside the probe event and re-derived from the journal on every drive round
(resume and crash recovery included, never a live re-probe — that would orphan
the session, ADR-002/#12); pre-#13 journals without the key declare nothing and
behave byte-identically. The clamp bounds the FLEET, not the plan: chunk plans
stay untouched because lease order is lowest-idx-first, so a single worker
walks a TUS session in ascending offset order. Evidence that shaped it: the
offline RED (2 concurrent calls against a declared-1 session) and the honest
live finding that tusd's per-upload lock happens to serialize concurrent
ascending chunks — server mercy, not protocol honesty; the engine obeys the
declaration regardless of which compliant server it meets.

## ADR-012 — Measure the link, declare the profile, plan from the journal ✅ accepted (issue #41, 2026-11-04)

The engine never guesses the link. The prober (`prober.rs`, #41) measures
the 1-vs-K question with the job's OWN bytes: a single-stream canary (S1),
then K independent tus sessions concurrently (SK — the session is
sequential, the link is what is measured; each stream carries 1/K so both
phases cost the same wall time). Classification: SK ≥ 2×S1 → PerFlowCap
(fleet = K); SK < 1.5×S1 → BearerCap (fleet = 1); between → Mixed (raise
cautiously). The recommendation (piece = 20 s of measured wire, clamped to
the lane floor #44 and the engine max) is DATA: the host journals it as a
`link_profile` event (ADR-002 — journaled, never re-probed mid-flight), and
the planner consumes journaled profiles. The prober's canary sizing is
rate-adaptive (no fixed 8 MiB probe — at 3 KB/s that is 45 minutes). Fleet
and piece consumers land with the M8 scheduler (docs/07 §3); what ships in
M7 is the measurement, the classification, the drills, and the journal
shape. Evidence: prober_drills.rs — per-flow caps (8 KiB/s per connection)
classify PerFlowCap with SK/S1 = 4.0×; a shared bearer (8 KiB/s TOTAL)
classifies BearerCap with SK/S1 = 1.0×.

## ADR-013 — Provider knowledge is DATA in the app; the engine stays provider-agnostic ✅ accepted (the destination lane, owner-ratified 2026-11-11)

The owner's finding: regular users think in websites ("Backblaze", "R2"),
not in path-style object URLs — they cannot obtain the endpoint/bucket/keys
the engine speaks, and cannot tell whether IUM would help them at all. The
decision, three layers deep:

1. **The registry is a JSON file** (`ium-app/src/data/providers.json`) —
   one source of truth carrying endpoint templates, region lists, key-page
   deep links and per-provider key recipes. NO provider code exists in the
   engine: the adapters stay pure protocol (ADR-011 untouched), and adding
   a provider is an edit to data, not to logic. The TS loader derives the
   URL; the Rust integration test (`tests/providers_registry.rs`) feeds
   every preset's sample URL through the ENGINE's own `parse_target` — the
   registry cannot drift into an endpoint the engine refuses, because both
   sides consume the same file and the parser is the single judge.
2. **The S3 Test button is an engine primitive** with the adapter's own
   probe-context taxonomy (`check_credentials`: zero-byte
   `CreateMultipartUpload` + best-effort `AbortMultipartUpload` on an
   `ium-probe/` key — never a user key, never completed, never orphaned).
   The shell's `probe_credentials` grows the s3 arm over it; the honest
   catch-all stays for anything still uncheckable.
3. **The shareable link is a presigned GET, minted on demand** —
   `sigv4::presign_url` (query-string SigV4, the S3 1..604800 s bound), the
   shell's `share_link` command rebuilding the object from the JOURNALED
   endpoint through the engine's parser (no UI-side URL truth), signed by
   the SAME credentials that uploaded. Nothing is ever auto-published; a
   link exists only when the user asks, and it expires (v1 pins 24 h).

Rejected alternatives, on the record: a relay service holding user
credentials (betrays the zero-telemetry posture — the keys-stay-local
promise is a selling point, not an implementation detail); per-website
proprietary adapters for "every website X" (an infinite maintenance set —
support things with real APIs, be honest about the rest); OAuth connectors
for the Drive/Dropbox/OneDrive tier (the eventual mainstream answer, but a
per-provider maintenance liability — deferred until the presets prove the
funnel). Evidence: tests/providers_registry.rs (7 presets × the real
parser), tests/sigv4_presign.rs (vectors minted by an independent Python
reference anchored on the published AWS Vector B), tests/s3_check_credentials.rs
(a loopback mock S3 pinning create→abort, 403/404/transport classification,
and the never-touched-a-user-key invariant), the vault schema test (the
endpoint field is optional, non-secret, and its note discloses exactly what
the probe touches).
