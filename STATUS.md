# IUM — STATUS (Single Source of Truth)

> **Standing rule (ADR-008, team decision 2026-11-02):** every task/step ends with this file
> updated and pushed to GitHub. It is the recovery point for a recycled sandbox. Read it FIRST
> in every session, before touching any file.
>
> Last updated: **2026-11-02** · Phase: **M2 — Hardening & Honesty** (M1 complete)

---

## 0. Bootstrap a fresh session (90 seconds)

```bash
cd /home/z/my-project
git pull                      # remote: github.com/kahlanhamza/IUM (private)
. scripts/rust-env.sh         # persistent toolchain in .toolchain/ (gitignored)
cargo test -p ium-core        # expect: 60 passed (37 unit + 5 loopback HTTP + 17 integration + 1 compile-fail contract)
bash scripts/gate.sh          # before ANY push: gates on the COMMITTED tree (defect #11 rule)
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

## 3. In progress

M3 spike **done**: design note `docs/07_M3_S3_Multipart_Spike.md` + milestone M3
filed with issues #6–#9 (sigv4 → adapter_s3 → MinIO harness/kill-9 drill; #8 planner
contract parallel). Spike findings already filed: `max_chunk_bytes` never consumed
(#8), 0-byte → PutObject path, verify-as-commit-gate (ADR-010 proposal).

Next: **#6 (sigv4)**.

## 4. Backlog / next (ordered — milestone [M3: Real transports](https://github.com/kahlanhamza/IUM/milestone/2))

1. **#6 — sigv4 signer** (pure, offline, official AWS test vectors) → **#7 — S3
   multipart adapter core** → **#9 — MinIO CI harness 3-OS + kill-9 drill + >100 MiB
   nightly e2e**; **#8** (planner consumes `max_chunk_bytes`, ADR-011) parallelizable
   anywhere.
2. TUS 1.0.0 adapter (next M3 tranche after S3 — reuses ADR-010 finalize pattern +
   CI harness shape).

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
fmt+clippy+test, refuses dirty trees)**; GitHub Actions CI (3-OS matrix) + nightly (fuzz +
coverage, dispatch-verified); GitHub REST API via PAT (issues/milestones/actions —
`scripts/gh_issues.py`, curl); Python 3 for repo scripts; tokio dev-dependencies: tempfile,
async-trait, serde_json, trybuild.

**Needed later:** MinIO container for S3-multipart tests (M3, via CI services or docker);
a TUS test server (M3); Tauri CLI + Node 20 (M4 — frozen). MCP servers are not installable in
this sandbox; the GitHub REST API covers issue/milestone/label management with the same effect.

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
| 2026-11-02 | **M3 spike done**: design note `docs/07` (protocol↔trait mapping, parts-atomic resume + ListParts rebuild, ADR-010 verify-as-commit-gate proposal, hand-rolled SigV4 vs SDK, MinIO single-binary on 3 OS — services containers are Linux-only); milestone M3 + issues #6–#9 filed idempotently (`scripts/file_m3.py`); roadmap M2 marked ✅, M3 amended | roadmap/STATUS aligned; M3 backlog live on GitHub | see git log |

## 10. Security

- Token state: fine-grained PAT restored into `.secrets/gh_token` on 2026-11-02 (gitignored,
  chmod 600). **It has now been pasted into chat twice — rotate it at
  <https://github.com/settings/personal-access-tokens> when convenient.** After rotating:
  `mkdir -p .secrets && chmod 700 .secrets && echo 'NEW_TOKEN' > .secrets/gh_token && chmod 600 .secrets/gh_token`
  (never commit this file; scopes needed: repo contents read/write, issues read/write,
  metadata read).
- `.secrets/` and `.toolchain/` are gitignored; `core.fileMode false` is set (sandbox mode churn).
