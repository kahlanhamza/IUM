# IUM — STATUS (Single Source of Truth)

> **Standing rule (ADR-008, team decision 2026-11-02):** every task/step ends with this file
> updated and pushed to GitHub. It is the recovery point for a recycled sandbox. Read it FIRST
> in every session, before touching any file.
>
> Last updated: **2026-11-02** · Phase: **M3 — Real transports** (M1+M2 complete; S3 tranche complete)

---

## 0. Bootstrap a fresh session (90 seconds)

```bash
cd /home/z/my-project
git pull                      # remote: github.com/kahlanhamza/IUM (private)
. scripts/rust-env.sh         # persistent toolchain in .toolchain/ (gitignored)
cargo test -p ium-core        # expect: 104 passed (56 unit + 23 adapter_s3 + 5 loopback HTTP + 18 integration + 1 kill-9 drill [env-gated] + 1 compile-fail contract)
bash scripts/gate.sh          # before ANY push: gates on the COMMITTED tree (defect #11 rule)
bash scripts/s3-test.sh       # live S3 suite vs pinned SeaweedFS 4.51 (downloads ~45 MB per run)
```

If `.toolchain/` was wiped: `bash scripts/install-toolchain.sh` (installs rustup minimal profile
into the project dir — never into `$HOME`, which the sandbox recycles).

## 1. What IUM is (30 seconds)

An upload manager that applies download-manager reliability engineering to uploads: dynamic
chunk pool, crash-safe SQLite WAL journal, counted retries with backoff, slow-connection
eviction, verify-before-declare, honest badges. Engine-first: `ium-core` (Rust, no UI);
the Tauri/React shell (`ium-app/`) is **frozen** until M2+M3 (ADR-006).

## 2. Done — verified by execution

| Date | Item | Evidence |
|---|---|---|
| 2026-11-01 | Sprint 0: all 7 audited core defects fixed (journal migrations via `PRAGMA user_version`, transactional job+chunk insert, loud write failures, re-rentable partial confirms, counted retries, journal-truth state machine, streaming HTTP adapter + verify gate) | commit `145c983`, `docs/03_Sprint0_Engine_Record.md` |
| 2026-11-01 | CI rebuilt: 3-OS matrix (ubuntu/macos/windows) running fmt + clippy `-D warnings` + tests; broken YAML (`branches: ain]`) repaired | `.github/workflows/ci.yml` |
| 2026-11-02 | Post-recycle audit: confirmed all 7 fixes at HEAD; ran full gates locally | audit trail §5 below |
| 2026-11-02 | **3 gate defects caught and fixed** (slipped in via ungated snapshot commit `0c11211`): clippy lint `eviction.rs:96`, invisible wedged calls in `ScriptedAdapter` log + wrong offset convention in stall test, fmt drift | commit `fix(core): restore green gates…` |
| 2026-11-02 | Gates verified green by execution: **54/54 tests, fmt, clippy `-D warnings`** | local run log, session log §9 |
| 2026-11-02 | Process infra: STATUS.md, roadmap, engineering process, ADRs, issue/PR templates, `scripts/gh_issues.py` | this commit |
| 2026-11-02 | **Issue #1 closed** the new way: design note + RED evidence on the issue, compile-fail contract test (trybuild), then fix. `verify` is now a required trait method — verify-before-declare cannot be silently inherited (ADR-004 fully enforced) | 55/55 gates green, [issue comment](https://github.com/kahlanhamza/IUM/issues/1#issuecomment-6563392514) |
| 2026-11-02 | **Defect #11 caught & fixed**: pushed HEAD `99ed179` was RED on CI — `d62094a` shipped the trybuild contract test but forgot to stage the `ium-core/Cargo.toml` dev-dependency; local gates had run on the working tree. Fix staged properly; permanent fix: `scripts/gate.sh` now gates the **committed tree** in a detached worktree | RED: E0433 reproduced on `99ed179` + CI failure confirmed via API; GREEN: gate.sh 55/55 on committed tree |
| 2026-11-02 | **Issue #2 closed** the new way: design note on the issue (0-rows legality table) + RED evidence (silent `Ok(())` on a corrupted journal) + fix `JournalError::ChunksMissing` rows-affected check, test pinned to the variant. Design note surfaced the planless-job hole → filed as #5 instead of scope-creeping | 56/56 gates green; #2 auto-closed by commit |
| 2026-11-02 | **Issue #4 closed**: fuzz is env-gated (`IUM_FUZZ_ROUNDS`/`IUM_FUZZ_OPS`), nightly workflow runs it at 400×2000 release + uploads an HTML coverage artifact (cargo-llvm-cov). Alarm proven by mutation: planted `complete()` off-by-one failed the expanded fuzz instantly; reverted, gates re-green. Local coverage baseline 90.78% regions / 93.55% lines | nightly.yml live; dispatch-verified (see issue #4) |
| 2026-11-02 | **Issue #5 closed**: invariant pinned — `plan(0) → [(0,0)]`, `insert_job`/`replace_chunks` reject empty plans (`EmptyPlan`), #2's special case deleted (0 rows is always `ChunksMissing`). Honest-badge hole closed: a planless job could reach Done with ZERO server round-trips (vacuous finish gate) — RED proved it live. 0-byte e2e on real HTTP loopback + mock: create → upload → verify → Done | 59/59 gates green; #5 auto-closed by commit |
| 2026-11-02 | **Issue #3 closed**: `MockAdapter` logs at entry (`bytes_moved: 0`), patched at completion — parity with `ScriptedAdapter`; blipped calls now visible (RED: 4 upload calls, log empty). `partial_first_len` first-call detection moved `is_empty()` → `len() == 1` so the entry-log doesn't silently disable the partial-confirm path. Tests-only change, no production code | 60/60 gates green; #3 auto-closed by commit |
| 2026-11-02 | **Issue #6 closed**: `sigv4` module — pure, offline, zero I/O. Five external vectors pinned AFTER triple-sourcing (official suite via aws-crt-nodejs, AWS S3 developer guide, suite-mirror creq→authz pairs) and reproduced by an independent Python reference first; a remembered kSigning constant failed that check and was discarded. Rule surfaced by vectors: `x-amz-content-sha256` signed only for `service == "s3"`. Clippy rejected the 9-arg sign → `SigningInput` struct, no allow-escapes. Deps: hmac/sha2/hex only, no AWS SDK | 69/69 gates green; #6 auto-closed by commit |
| 2026-11-02 | **Issue #7 closed**: `adapter_s3` — first real transport. Protocol rehearsed LIVE (independent Python signer) before Rust: MinIO dead (repo archived 2026-04-25, dl.min.io 410) → tests are server-agnostic env-gated; rehearsal ran vs Garage v2.1.0 and captured wire facts (lowercase `etag` header, `&quot;` XML entities, NoSuchUpload-on-ListParts/Complete idempotency) now pinned as tests. Session state: UploadId lives in adapter map, journaled via probe event (ADR-002), `restore_session` = reattach path; map miss → SessionLost → Reattach. Parts atomic: body = [begin,end) full span; verify = ADR-010 finalize-if-needed (reconcile 1..=k contiguous + Σ==source BEFORE Complete — never completes a wrong assembly) → HEAD. Clippy: 9-arg `call` → `S3Request` struct, no allow-escapes. **ADR-010 accepted** | 93/93 gates green, 20/20 LIVE vs real S3 server incl. restart drill (wire counter: only the missing part moved) + verify-refuses-to-lie; #7 auto-closed by commit |
| 2026-11-02 | **Issue #8 closed**: planner consumes `ProbeReport.max_chunk_bytes` (ADR-011 accepted: adapter declares, planner obeys). RED at `69d2686`: `cap 4 MiB on 10 MiB must raise n to >= 3, got 2`. `ChunkPool::plan_capped` composes the cap as a HARD bound — `n = max(min(n, ⌊total/chunk_min⌋), ⌈total/cap⌉)` — the existing chunk_min clamp can never undo a cap raise; when `cap < chunk_min` the cap wins (a violated limit fails uploads, a violated floor only costs requests). Manager re-plans at the probe boundary through the journal (`replace_chunks` + `replan_cap` event), only while every chunk is Pending/0-confirmed (journal-verified), resumable endpoints only. Not-binding = byte-identical to `plan` | 100/100 gates green (56 unit + 18 integration incl. cap test); #8 auto-closed by commit |
| 2026-11-02 | **Issue #9 closed**: S3 CI harness on 3 OS + kill-9 drill + >100 MiB nightly e2e. Server re-decided: **SeaweedFS 4.51 pinned** (Garage v2.1.0 ships Linux-only official binaries; one product for linux/darwin/windows from GitHub Releases with md5 sidecars). The pin caught a real bug on day one: SeaweedFS emits NUMERIC character references (`&#34;`) where Garage emits named (`&quot;`) → adapter's named-only decode left literal `&#34;` in etags → `Complete` answered `InvalidPart` (RED: 18/20 live). Fixed: `xml_text` = single-pass XML entity decode (named + numeric dec/hex, double-decode-safe), both servers' captured vectors pinned as tests. Harness `scripts/s3-test.sh`: per-OS pinned download + md5 verify, random free port triple (no stale-server hazards, git-bash-portable), throwaway creds via `s3.json`, bucket via `scripts/s3curl.py` (stdlib SigV4 from #6's independent reference), env-gated suite, abort-leftover-multiparts on exit (trap). Kill-9 drill: `src/bin/s3_drill_child.rs` (upload/resume modes, ticket-gated adapter wedges calls at ENTRY — deterministic mid-body state) + `tests/s3_drill.rs` (parent): 112 MiB → first part confirmed → SIGKILL → fresh process reattaches from journaled probe event → `WIRE == 112 MiB − 28 MiB` (zero re-upload) → honest Done after ADR-010 verify. ci.yml: S3 step on all 3 OS; nightly: `s3_drill` job (Linux) | 104/104 gates; LIVE: 23/23 suite + drill vs SeaweedFS 4.51 (`WIRE 88080384 < 117440512`); #9 auto-closed by commit |

## 3. In progress

Next: **TUS 1.0.0 adapter** (next M3 tranche — reuses the ADR-010 finalize pattern,
the `s3-test.sh` harness shape, and ADR-011's declared-cap contract). M3 remaining
after S3: nothing else open — #6 #7 #8 #9 all closed.

## 4. Backlog / next (ordered — milestone [M3: Real transports](https://github.com/kahlanhamza/IUM/milestone/2))

1. ~~#6 — sigv4 signer~~ ✅ (2026-11-02).
2. ~~#7 — S3 multipart adapter core~~ ✅ (2026-11-02, ADR-010 accepted).
3. ~~#8 — planner consumes `max_chunk_bytes`~~ ✅ (2026-11-02, ADR-011 accepted).
4. ~~#9 — S3 CI harness 3-OS + kill-9 drill + >100 MiB nightly e2e~~ ✅ (2026-11-02,
   SeaweedFS 4.51 pinned; MinIO premise dead, Garage Linux-only).
5. TUS 1.0.0 adapter (next M3 tranche — reuses ADR-010 finalize pattern + harness shape).

## 5. Defect audit trail

Sprint-0 audit (7 defects — **all resolved at `145c983`**, verified 2026-11-02):

| # | Defect | Status | Where |
|---|---|---|---|
| 1 | No schema migration (`PRAGMA user_version`) | FIXED | `journal.rs` `MIGRATIONS` + `migrate()` |
| 2 | Silent no-op UPDATE/DELETE | FIXED | `set_job_state`/`update_chunk` fail loudly |
| 3 | Job+chunks insert not atomic | FIXED | `insert_job` single transaction |
| 4 | Partial confirm → chunk never re-leased → premature Done | FIXED | manager marks `Pending`, keeps bytes; `finish()` consistency gate |
| 5 | Retry attempts never counted → infinite retry | FIXED | per-chunk attempt map, exhaustion → Failed |
| 6 | State machine trusted caller args, not journal | FIXED | `transition()` re-reads journal state |
| 7 | Adapter not streaming / no verify gate | FIXED | `adapter_http.rs` streams from disk; HEAD verify gate |

2026-11-02 additions (found by gates, fixed same day):

| # | Defect | Root cause | Fix |
|---|---|---|---|
| 8 | clippy `manual_is_multiple_of` hard error (`eviction.rs:96`) | ungated commit | `is_multiple_of(2)` |
| 9 | `ScriptedAdapter` logged calls only at completion → wedged calls invisible; stall test asserted chunk-relative offsets while `Lease.resume_from` is absolute | ungated commit; test never run | log at entry, patch `bytes_moved` at completion; assert vs `snap.chunks[1].begin` |
| 10 | fmt drift (`manager.rs`, `integration.rs`) | ungated commit | `cargo fmt` |
| 11 | Pushed HEAD red on CI: contract test committed without its `trybuild` dev-dependency (`d62094a` forgot to stage `ium-core/Cargo.toml`); CI failure on `99ed179` | gates ran on the working tree, not the committed tree — unstaged files are invisible to them | fix commit `6423227` stages the dep; `scripts/gate.sh` gates HEAD itself in a detached worktree and refuses dirty trees |

**Lesson (ADR-007):** all three came from ONE commit that bypassed the gates. Gates are now
non-bypassable — no commit lands without fmt + clippy + test green locally (CI re-checks on 3 OS).
**Lesson (defect #11):** passing gates must be measured on what you push. Working-tree gates
proved nothing about the committed tree; `gate.sh` closes that hole by construction.

## 6. Tools — used / needed

**Used now:** git; Rust stable 1.99.1 (rustup minimal profile + rustfmt/clippy, persisted in
`.toolchain/`, activated via `scripts/rust-env.sh`); cargo fmt/clippy/test; **cargo-llvm-cov
0.9.1 (coverage, nightly job)**; **`scripts/gate.sh` (committed-tree gate: detached-worktree
fmt+clippy+test, refuses dirty trees)**; GitHub Actions CI (3-OS matrix + S3 wire suite via
`scripts/s3-test.sh`) + nightly (fuzz + coverage + kill-9 drill, dispatch-verified); GitHub
REST API via PAT (issues/milestones/actions — `scripts/gh_issues.py`, `scripts/gh_comment.py`,
curl); Python 3 for repo scripts; tokio dev-dependencies: tempfile, async-trait, serde_json,
trybuild. **S3 test harness (#9)**: pinned SeaweedFS 4.51 per-OS (downloaded fresh each run,
md5-verified, throwaway creds, random ports) + `scripts/s3curl.py` (stdlib SigV4 client:
bucket create, header debug, abort-multipart cleanup). Local rehearsal binaries under
`.weed-local/` / `.garage-local/` (gitignored, throwaway) — not repo dependencies.

**Needed later:** a TUS test server (M3 — same harness pattern as s3-test.sh); Tauri CLI +
Node 20 (M4 — frozen). MCP servers are not installable in this sandbox; the GitHub REST API
covers issue/milestone/label management with the same effect.

## 7. Architecture (one screen)

```
ium-app (Tauri 2 + React)          FROZEN (ADR-006)
   │ commands/FFI (later)
ium-core
   ├─ manager.rs    JobManager — supervisor state machine; journal is the only truth
   ├─ journal.rs    SQLite WAL, versioned migrations, atomic inserts, loud write failures
   ├─ chunk.rs      ChunkPool — dynamic segments, leases, re-rentable partial confirms
   ├─ retry.rs      counted attempts, backoff, Retry-After, exhaustion → Failed
   ├─ eviction.rs   throughput watch, below-floor strikes → lease eviction (spec §4.2)
   ├─ adapter.rs    TransferAdapter trait + registry (+ verify gate types)
   ├─ adapter_http.rs  HttpOneShotAdapter — reqwest+rustls, streams from disk, HEAD verify
   ├─ adapter_s3.rs    S3MultipartAdapter — SigV4 multipart, ADR-010 commit-gate verify, restore_session reattach
   └─ state.rs, ids.rs
```

State machine: `Created → Probing → Running ⇄ Paused → Finishing → Done | Failed`;
every transition journaled **before** it executes; crash recovery rebuilds from the journal.
Invariants, design details: `docs/02_IUM_MVP_Spec_v1.md`, `docs/03_Sprint0_Engine_Record.md`.

## 8. Doc index

| File | What it is |
|---|---|
| `docs/01_Architecture_Teardown_v2.md` | Clean-room teardown of IDM/FDM upload behavior (evidence: `rea-lab/out/`) |
| `docs/02_IUM_MVP_Spec_v1.md` | The MVP spec — behavior contract the tests enforce |
| `docs/03_Sprint0_Engine_Record.md` | Sprint 0 record: audit findings + fixes |
| `docs/04_Roadmap.md` | Milestones M0–M5 with acceptance criteria |
| `docs/05_Engineering_Process.md` | How we work: issue → failing test → fix → gates → STATUS → push |
| `docs/06_Decisions_ADR.md` | Architecture decision records (ADRs 001–009) |
| `STATUS.md` | This file — update at the end of EVERY task |

## 9. Session log (append one row per task)

| Date | Task | Outcome | Commit |
|---|---|---|---|
| 2026-11-01 | Sprint 0: fix 7 audited defects, real HTTP adapter, CI matrix | 45 tests green, CI repaired | `145c983` |
| 2026-11-01 | Session snapshot (no message, gates bypassed) | introduced defects 8–10 | `0c11211` |
| 2026-11-02 | Post-recycle audit + gate fixes (defects 8–10) | 54/54 green locally, fmt+clippy clean | `adb9390` |
| 2026-11-02 | Process adoption: STATUS.md SSOT, roadmap, process, ADRs, templates | issue-driven loop now mandatory | `fca56c9` |
| 2026-11-02 | Filed milestone M2 + issues #1–#4 via `scripts/gh_issues.py` | backlog tracked on GitHub, not in chat | `adb8fa0` |
| 2026-11-02 | Issue #1 via the full loop (design note → red → fix → green → close) | verify now required; trybuild contract test; 55/55 | see git log |
| 2026-11-02 | **Defect #11**: audit caught pushed HEAD `99ed179` red on CI (unstaged trybuild dep); RED reproduced locally, fix committed properly | pushed tree now compiles: 55/55 | `6423227` |
| 2026-11-02 | Process hardening: `scripts/gate.sh` — gates run on the committed tree (detached worktree), dirty tree refused; process doc + DoD updated | gate.sh GREEN on committed tree; this STATUS row is the record | `ca3b3eb` |
| 2026-11-02 | Issue #2 via the full loop (design note → RED → fix → GREEN); follow-up #5 filed; toolchain installer now ships rustfmt+clippy (recycle gap) | 56/56 gates green on committed tree; #2 closed | `b29b8e9` + `cae3be5` |
| 2026-11-02 | Issue #4 via the full loop (design note → mutation-RED → revert → workflow → dispatch-verified) | nightly fuzz 400×2000 + coverage artifact; alarm proven to ring; 56/56 | `ecb3374` |
| 2026-11-02 | Issue #5 via the full loop (design note → 4×RED → fix → 59/59); old-convention tests updated, not drive-bys | planless-job invariant pinned; honest-badge hole closed | `8b0c5fc` |
| 2026-11-02 | Issue #3 via the full loop (design note → RED → fix → 60/60); MockAdapter entry-logging parity, `is_empty()` → `len() == 1` subtlety documented on the issue | M2 milestone complete: #1 #2 #3 #4 #5 all closed | `d73147c` |
| 2026-11-02 | **M3 spike done**: design note `docs/07` (protocol↔trait mapping, parts-atomic resume + ListParts rebuild, ADR-010 verify-as-commit-gate proposal, hand-rolled SigV4 vs SDK, MinIO single-binary on 3 OS — services containers are Linux-only); milestone M3 + issues #6–#9 filed idempotently (`scripts/file_m3.py`); roadmap M2 marked ✅, M3 amended | roadmap/STATUS aligned; M3 backlog live on GitHub | `afefc93` |
| 2026-11-02 | Issue #6 via the full loop (design note + vector triple-sourcing → 9×RED on stubs → implement → 69/69); independent Python reference gate before pinning constants | sigv4 module green on official AWS vectors; deps hmac/sha2/hex only | see git log |
| 2026-11-02 | Issue #7 via the full loop (design note → protocol rehearsal vs Garage → compile-RED → implement → 93/93 + 20/20 LIVE); MinIO-archived finding recorded in docs/07 §6 amendment; ADR-010 accepted; #9 server premise amended | first real transport works live: multipart lifecycle, restart drill (zero re-upload), verify-refuses-to-lie | see git log |
| 2026-11-02 | Issue #8 via the full loop (design note → RED `got 2` → `plan_capped` + journal re-plan → 100/100); ADR-011 accepted | adapter cap is a hard planner bound; not-binding = byte-identical | `80c22c1` |
| 2026-11-02 | Issue #9 via the full loop (design note → server re-decision → LIVE RED 18/20 `InvalidPart` → `xml_text` single-pass entity decode → harness + drill + CI/nightly wiring → 104/104 + LIVE 23/23 + drill); SeaweedFS 4.51 pinned (Garage Linux-only); wire shapes of BOTH servers pinned as vectors | 3-OS S3 gate live; kill-9 drill: SIGKILL mid-body → zero re-upload → verify→Done | see git log |

## 10. Security

- Token state: fine-grained PAT restored into `.secrets/gh_token` on 2026-11-02 (gitignored,
  chmod 600). **It has now been pasted into chat twice — rotate it at
  <https://github.com/settings/personal-access-tokens> when convenient.** After rotating:
  `mkdir -p .secrets && chmod 700 .secrets && echo 'NEW_TOKEN' > .secrets/gh_token && chmod 600 .secrets/gh_token`
  (never commit this file; scopes needed: repo contents read/write, issues read/write,
  metadata read).
- `.secrets/` and `.toolchain/` are gitignored; `core.fileMode false` is set (sandbox mode churn).
