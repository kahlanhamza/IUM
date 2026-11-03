# IUM — STATUS (Single Source of Truth)

> **Standing rule (ADR-008, team decision 2026-11-02):** every task/step ends with this file
> updated and pushed to GitHub. It is the recovery point for a recycled sandbox. Read it FIRST
> in every session, before touching any file.
>
> Last updated: **2026-11-04** · Phase: **M6 COMPLETE-PROBE LANES GREEN — run #75 is the first ALL-GREEN CI since #61 (8/8 jobs: fmt, core ×3 OS, ium-app ×3 OS, app-e2e). The red streak #62–#74 is fully accounted: every failure audited from raw logs, root-caused, fixed by its own commit (#63/#64 core lanes; #65–#74 the app-e2e leg's seam-by-seam shakedown ending in the recorder's record-at-boot, `dd046a7`), and the final fix was verified locally (committed-tree gate + wire-level smoke: boot-record exists, HEAD 404 pre-PUT, aborted-PUT leaves zeros) before ONE push. #35 (real-shell WebDriver e2e, both legs proven on CI) and #37 (CI debt ledger) CLOSED with evidence comments. All issues #1–#40 closed; no open CI debt.**

---

## 0. Bootstrap a fresh session (90 seconds)

```bash
cd /home/z/my-project
git pull                      # remote: github.com/kahlanhamza/IUM (private)
. scripts/rust-env.sh         # persistent toolchain in .toolchain/ (gitignored)
cargo test -p ium-core        # expect: 143 passed (65 unit + 23 adapter_s3 + 12 adapter_tus + 5 loopback HTTP + 22 integration + 4 supervisor + 3 real-data drill [HTTP live; TUS/S3 env-gated] + 1 slow-connection eviction drill + 1 adaptive-resplit drill [#27] + 1 s3 kill-9 drill [env-gated] + 1 tus kill-9 drill [env-gated] + 1 compile-fail contract + …)
bash scripts/gate.sh          # before ANY push: gates on the COMMITTED tree (defect #11 rule)
bash scripts/s3-test.sh       # live S3 suite vs pinned SeaweedFS 4.51 (downloads ~45 MB per run)
                              #   --with-real: also the real-data drill (#17)
bash scripts/tus-test.sh      # live TUS suite vs pinned tusd 2.10.1 (downloads ~27 MB per run)
                              #   --with-drill: also the kill-9 TUS drill (nightly)
                              #   --with-real: also the real-data drill (#17)
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
| 2026-11-02 | **Issue #9 closed**: S3 CI harness on 3 OS + kill-9 drill + >100 MiB nightly e2e. Server re-decided: **SeaweedFS 4.51 pinned** (Garage v2.1.0 ships Linux-only official binaries; one product for linux/darwin/windows from GitHub Releases with md5 sidecars). The pin caught a real bug on day one: SeaweedFS emits NUMERIC character references (`&#34;`) where Garage emits named (`&quot;`) → adapter's named-only decode left literal `&#34;` in etags → `Complete` answered `InvalidPart` (RED: 18/20 live). Fixed: `xml_text` = single-pass XML entity decode (named + numeric dec/hex, double-decode-safe), both servers' captured vectors pinned as tests. Harness `scripts/s3-test.sh`: per-OS pinned download + md5 verify, random free port triple (no stale-server hazards, git-bash-portable), throwaway creds via `s3.json`, bucket via `scripts/s3curl.py` (stdlib SigV4 from #6's independent reference), env-gated suite, abort-leftover-multiparts on exit (trap). Kill-9 drill: `src/bin/s3_drill_child.rs` (upload/resume modes, ticket-gated adapter wedges calls at ENTRY — deterministic mid-body state) + `tests/s3_drill.rs` (parent): 112 MiB → first part confirmed → SIGKILL → fresh process reattaches from journaled probe event → `WIRE == 112 MiB − 28 MiB` (zero re-upload) → honest Done after ADR-010 verify. ci.yml: S3 step on all 3 OS; nightly: `s3_drill` job (Linux) | 104/104 gates; LIVE: 23/23 suite + drill vs SeaweedFS 4.51 (`WIRE 88080384 < 117440512`); CI 3-OS green incl. Windows; nightly `s3_drill` dispatch-verified; #9 auto-closed by commit |
| 2026-11-02 | **Issue #10 closed**: TUS 1.0.0 adapter — byte-level resume. Protocol rehearsed LIVE on tusd v2.10.1 (204 + Upload-Offset acks, 409 offset conflict, 404 bogus session, 412 without Tus-Resumable, NO checksum extension). `adapter_tus`: probe = POST creation (RFC 3986 §5.2 Location resolution); PATCH `[resume_from, end)` streamed from disk; `bytes_confirmed` = server-acked delta ONLY; **409 realign** absorbs drifted-but-server-acked bytes; verify = READ-ONLY HEAD (no commit gate — contrast ADR-010). Harness `scripts/tus-test.sh` (pinned tusd, sha256-verified, random port); ci.yml TUS step on all 3 OS | 115/115 gates; 11/11 LIVE vs tusd incl. 409-realign + byte-level restart drill; #10 auto-closed by commit |
| 2026-11-02 | **Issue #12 closed**: `Action::Reattach` implemented — the taxonomy's promise kept. `TransferAdapter::reattach(endpoint, token)` is a REQUIRED trait method (no default; trybuild contract regenerated: E0046 lists `verify`, `reattach`). S3/TUS delegate to inherent `restore_session` (map insert, no network, never re-probes — re-probe would orphan the live session); HTTP + doubles explicit no-ops; drill child delegates. Manager: mid-transfer Reattach replays the LAST journaled probe token, journals `reattach`, counts the attempt (same budget as Retry — really-gone session exhausts into Failed, recovery cannot spin); `finish()` opens with the same replay (`finishing_entry`) so ANY fresh process verifies on a journal-reconciled session — the stronger statement of ADR-002. Honest mock (`has_session` starts false, sessionless verify = SessionLost) exposed a third hole for free: the two pre-existing crash-recovery drills also died at verify in a fresh process — fixed by the same Finishing-entry replay; their fixtures now journal the probe event (a mid-flight job without one is impossible) | 118/118 gates (5×RED → green incl. the issue's scratch test); #12 auto-closed by commit |
| 2026-11-02 | **Issue #11 closed**: TUS kill-9 nightly drill — the s3_drill shape with the sharper window the issue names. `src/bin/tus_drill_child.rs`: upload mode gates every chunk but 0 at entry, and the wrapper **holds chunk 0's server-acked confirmation for 8 s** before handing it to the manager — the parent SIGKILLs inside a window that is provably real (journal `Running` + 0 confirmed while tusd holds 20 MiB). Resume child: FRESH adapter with an EMPTY session map — recovering the journaled creation URL through the manager's `Action::Reattach` (#12: map-miss → SessionLost → token replay) is part of the proof; the re-lease then absorbs the full-chunk drift through the 409 HEAD-realign (#10) with zero re-sent bytes. Chunk ordering is drill-local (TUS = one sequential-offset resource). Parent: `SERVER_ACK == SIZE/2` exact, journal 0 at kill, `SERVER_OFFSET == SIZE/2`, `DONE done` via read-only HEAD verify, byte-level zero re-upload pinned `SIZE/2 ≤ WIRE ≤ SIZE/2 + 64 KiB` (one rejected 409 attempt's buffered body chunk is the only honest slack — exact `==` races on hyper's first body poll). Nightly `tus_drill` job (Linux) via `scripts/tus-test.sh --with-drill`; YAML `safe_load`-validated | 119/119 gates; LIVE 11/11 suite + drill GREEN vs tusd 2.10.1 first attempt; #11 auto-closed by commit |
| 2026-11-02 | **Issue #13 closed** (first M4 item): `ProbeReport.max_parallel` — adapter declares CONCURRENCY on the ADR-011 channel, planner obeys (ADR-012 accepted). RED (offline, deterministic): declared-1 session saw **2 concurrent upload calls** (chunk 0 wedged at entry on a Notify; worker 2 leased chunk 1 mid-wedge). Honest live finding recorded on the issue: tusd 2.10.1's per-upload lock serializes concurrent ascending-offset PATCHes, so the live 16 MiB/max_conns=4 job passed even pre-fix — server mercy, not protocol honesty; the engine now obeys the declaration regardless of server. Fix: drive round runs `min(pending, MAX_WORKERS, declared_max_parallel)`; declaration journaled in the probe event, re-derived from the journal every round (resume/crash-recovery included, never a live re-probe); TUS declares `Some(1)`, S3/HTTP `None`; pre-#13 journals byte-identical; fleet-clamp only — plans untouched (lease order = lowest-idx-first walks TUS in offset order) | 121/121 gates; LIVE 12/12 vs tusd post-fix; #13 auto-closed by commit |
| 2026-11-02 | **Issue #14 closed** (unfreeze): ADR-006 amended — M2+M3 done, the shell joins the gates. `ium-app/src-tauri` is a workspace member; ci.yml's placeholder `scaffold` job replaced by a real 3-OS `ium-app` job (node 20 → `npm ci && npm run build` → `cargo clippy -p ium-app --all-targets -D warnings` → `cargo test -p ium-app`; ubuntu apt-installs WebKitGTK dev libs). RED captured honestly: the shell had NEVER compiled — `cargo check -p ium-app` fails on missing `webkit2gtk-4.1`/`gdk-3.0` system libs, and this sandbox has no root to install them (windows-msvc cross-check also impossible: no `lib.exe` for cc-rs). Local gate therefore: frontend lane (tsc strict + vite) UNCONDITIONAL — the sandbox verifies it fully; shell cargo lane capability-gated with a LOUD skip + CI owns it natively on every push (same honesty contract as env-gated S3/TUS suites, recorded in ADR-006 amendment). Icons generated by script (`scripts/make_app_icons.py`, PIL geometric drawing, no vendor assets); `package-lock.json` committed. CI verified GREEN on all 3 OS incl. the first-ever shell compile | gate GREEN on committed tree (frontend lane real, shell lane loud-skip→CI); #14 auto-closed by commit |
| 2026-11-02 | **Issue #15 closed** (runtime wiring): `ium_core::supervisor::Supervisor` — spawn/pause/resume/cancel over `Arc<JobManager>`, engine-side so every semantic is testable without a webview; Tauri glue stays thin. `JobState::Canceled` first-class terminal (never Failed — honest endings), legal Created/Probing/Running/Paused, NOT Finishing; journaled BEFORE the drive task aborts (ADR-002); survives reopen; drive-loop top checks cancel FIRST; `transition_or_canceled` absorbs cancel racing any in-flight transition. RED: compile-fail (E0432 supervisor, E0599 Canceled) posted on the issue. **Corruption catch earned by the tests**: `journal::parse_state` had `_ => JobState::Created` — an unknown state string silently resurrected any job as fresh (a canceled job would come back new!). Fixed loudly: exhaustive parse + `FromSqlConversionFailure` on unknown values. Shell: Engine holds the Supervisor; commands create_job/drive/watcher, resume re-drives, cancel_job, inspect_chunks, stat_file; adapters registered at setup (s3 only when env creds exist); `job_updated` watcher emits journal snapshots until the terminal one | 126/126 gates (121→126: +4 supervisor, +1 state cancel test); CI `ium-app` job compiles the glue on 3 OS; #15 auto-closed by commit |
| 2026-11-02 | **Issue #16 closed** (Sprint-1 UI slice, spec §8): `logic.ts` — the UI's brain as PURE functions over journal snapshots (badge verbatim + "unknown" for unknown capabilities; canPause/canResume/canCancel mirroring `state.rs` exactly: pause = non-terminal, resume = paused only, cancel = created/probing/running/paused, never finishing; confirmedTotal = Σ server-confirmed only; 0-byte = 100% only when done; speed = Δconfirmed/Δt display math). RED posted (vitest wired, `./logic` unresolvable) → 9/9 GREEN; one test-side arithmetic slip caught and fixed (100 B over 0.5 s = 200 B/s — the implementation was right, the expectation wasn't; recorded on the issue). Components dumb (props in, events out): `CreateJobForm` (official dialog plugin file picker → `stat_file` → engine plans; adapter select from `list_adapters` — reflects the registry, never hard-codes), `JobsTable` (badge/state chips, progress bar, live speed, engine-gated Pause/Resume/Cancel), `InspectorDrawer` (per-chunk table + journal event list, aria2-style transparency). Shell: `list_adapters` command + `tauri-plugin-dialog` registered. vitest added to the gate's frontend lane and CI's `ium-app` job — the sandbox now REALLY tests UI logic locally | vitest 9/9 + tsc strict + vite build (152.65 kB) in the sandbox; shell lane compile-verified by CI 3-OS; #16 auto-closed by commit |

## 3. In progress

**M5 — API freeze, docs & real-data evidence** (owner mandate 2026-11-02 folded in:
"always test on real data" — standing rule in docs/05 "Real data only"). Milestone 4
filed on GitHub with four issues, ALL CLOSED: #17 (real video through HTTP+TUS+S3),
#18 (real throttled socket, live eviction), #19 (API freeze: surface shrunk ~26 items,
`#![deny(missing_docs)]`, cargo doc lane in gate + CI), #20 (semver policy + CHANGELOG
discipline). M5 milestone complete per docs/04 — every box checked.

**M6 — Production readiness: the probe generation** (milestone 5, filed 2026-11-03 after
#31's boundary register predicted exactly which seams a new probe class touches first).
✅ #32 closed (2026-11-03, local + cloud run 41654248875: loss stalls absorbed with zero
false evictions; 250 ms RTT never churned; a real mid-body RST lands as ONE counted
`chunk_error {kind: Transient, action: Retry}` — and the probe's own assert caught the
harness leaving the victim model armed on the rtt leg, engine innocent). ✅ #33 closed
(2026-11-03, suite green against THREE S3 implementations (SeaweedFS 4.51, Garage
rehearsal, rclone serve s3 1.75.2 — 23/23) and TWO TUS implementations (tusd 2.10.1,
@tus/server 2.4.7 — 12/12 with one loud recorded delta); the probe's first rclone run
caught #36, a latent real-AWS defect the canonical server masked, fixed same session).
✅ #34 closed (2026-11-03 — the journal seam: atomic `set_job_state_event`, honest
Paused park on mid-transfer ENOSPC, half-transition RED closed, real-OS EFBIG pin via
RLIMIT_FSIZE, soak bounds pinned (16,391 events → 2.4 MB / WAL→0 / RSS +5.5 MiB),
`--with-longhaul` leg live vs SeaweedFS with honest timebox-pause + fresh-journal
replay byte-exact).
Open: #35 (real-shell WebDriver e2e). Owner's feature
tranche (WebDAV/Drive, OAuth, Sprint-2 UI) decides after M6.

CI debt (#37, REOPENED 2026-11-04 by the owner — runs #62–#69 all failed while new
pushes kept landing): the 2026-11-03 closure was premature. Audit (posted on the
issue): #62 macos = concurrency-cancel phantom; #63 = windows rlimit compile (fixed
in-tree) + bogus-session flake (fixed by the three-way probe); #64 = realign flake
(fixed by settled arithmetic); **#65–#69 = app-e2e failed 5× IDENTICALLY** — not
flaky but a design flaw: leg A ran the registry default `http_oneshot`, which plans
ONE chunk and journals `bytes_confirmed` only at PUT completion, so the UI's honest
Σ-confirmed percent cannot move mid-transfer (12 s of 0% then 100% under the
throttle) and `p2 > p1` was unsatisfiable by construction. Fix: loopback-recorder
gains a real TUS lane (`POST /files` creation → relative `Location`, throttled
`PATCH` → `204` + server-acked `Upload-Offset`, `HEAD` truth, 409/412/404 shapes —
12/12 of the adapter's own wire suite passes against it); e2e leg A selects `tus`
(W3C execute-sync, native value setter + real change event — WebKitGTK's select
popup is unreachable under xvfb), sets 6 connections (5 chunks ≥ 5 MiB floor), and
the live-progress assert polls with a 45 s deadline instead of two blind samples;
leg B stays one-shot for the honest cancel-nothing semantics. Flake faces verified
dead: `scripts/stress_tus_flakes.sh` = 36/36 green (12× × recorder + @tus/server
2.4.7 + tusd 2.10.1). Offline suite 150/150, fmt + clippy `-D warnings` clean.
#37 closes when this push's CI run is green; #35's lane rides the same commit.

CI debt FINAL face (issue #39, filed 2026-11-04 after run #70): #70 was the FIRST
ci run in history whose e2e actually reached the Upload click (leg A's tus rework
fixed the submit-enable blocker of #65–#69) — and the shell DIED on it:
`Session terminated without a reply` on the click, driver.log ending with a Rust
backtrace and "thread caused non-unwinding panic. aborting.". Root cause (tauri v2
docs, quoted on the issue): SYNC commands run on the GTK main thread, where no
tokio context is entered; `Supervisor::drive`'s raw `tokio::spawn` panicked there
and the unwind crossed the webview IPC FFI boundary → abort → session gone.
Unit tests never saw it: they all construct/drive inside `#[tokio::test]`. RED
reproduced in `ium-core/tests/supervisor.rs::drive_is_callable_off_the_runtime_thread`
(panic "there is no reactor running, must be called from the context of a Tokio
1.x runtime" at supervisor.rs:77 — posted on the issue); fix: `Supervisor` captures
a `tokio::runtime::Handle` at construction, `with_handle` for context-less hosts,
`drive()` spawns through the stored handle (any thread), honest panic only when a
host constructs context-less AND gives no handle; ium-app setup passes tauri's
async-runtime handle. Second test pins the shell's exact shape (construct AND drive
off-runtime → Done). 87/87 suite green, fmt + clippy `-D warnings` clean. Same
commit: ci.yml `cancel-in-progress: false` (the #62 macos phantom-red can never
recur — red must mean RED), and app-e2e forensics `tail -200` (the 25-line tail
cut off the panic message and cost a diagnosis cycle).

Run #71 (@ `940ddc9`) VERDICT: 7/8 jobs green FIRST TRY (core ×3 OS + ium-app ×3 OS
+ fmt — the with_handle wiring compiles everywhere); app-e2e PROVED #39's fix by
wire truth (session survived the click, recorder `tus_sessions: 1, tus_bytes:
25165824` — the full 24 MiB through the real shell) and caught the NEXT seam: the
row froze at `probing pct=0` for the whole 30 s poll — the UI never received a
single `job_updated` snapshot → **issue #40**: no `capabilities/` file existed, and
tauri v2's ACL grants NOTHING by default to core-plugin commands (docs: app
commands "are allowed to be used by all the windows" by default — that's why every
invoke worked — but `plugin:event|listen` is gated behind `core:event:default`).
Fix: `ium-app/src-tauri/capabilities/default.json` — `core:default` (event stream
+ window/path/app defaults) + `dialog:default` (the picker; the paste seam had been
dodging it). CHANGELOG entries for #39 + #40 in the same commit.

Run #72 (@ `62444ee`) VERDICT: the capabilities fix WORKED — leg A is fully green
through the real shell (recorder `tus_bytes: 25165824` again; running → live
progress → pause → resume → Done all passed) — and died one step later clicking
the row for the inspector: `element not interactable`. Mechanism (posted on #35):
the jobs table is wider than the window (7 columns + 3 action buttons), a
full-width `<tr>` cannot be scrolled into view horizontally and the click atom
refuses it (geckodriver#1294 = same class). Fix is harness-only: click the row's
first CELL (onClick is on the `<tr>`, every cell bubbles — same user path). Chain
so far: #70 abort at click (#39) → #71 frozen row (#40) → #72 row click (#35
 Chain
so far: #70 abort at click (#39) → #71 frozen row (#40) → #72 row click (#35
harness fix).

Run #73 (@ `ebe2c3e`) VERDICT: the cell click WORKED — the inspector opened and
leg A is fully green including the chunk-table asserts — and leg B died at the
running-wait: `last state=done pct=100`. Recorder B: `put_count: 1,
received_bytes: 16777216` — the PUT ran honestly at its 1 MiB/s throttle
(16 MiB ≈ 16 s, inside the 30 s window). The test watched THE WRONG ROW:
`list_jobs` renders NEWEST-FIRST, so the moment job B was created the table
re-ordered to [B, A] and `tr:last-child` — written in the one-row world, never
executed until now — pointed at job A. Fix: `row_for_file()` addresses rows BY
CONTENT (first cell holds the basename), legs A and B both; zero positional
selectors remain. Side note: #73's `core (macos-latest)` = cancelled with a
missing log blob — runner died mid-job,  Run
#74 (@ `97eea69`): the content-addressed rows worked — leg B ran, Cancel clicked,
`Canceled` badge PASSED — and died at the last server-truth read: `record file
recorder-b.json never appeared`. Mechanism: the cancel fires the moment `running`
shows (the one-shot family's confirmed bytes cannot move mid-flight), which can
beat the PUT's first byte; the recorder saw ZERO requests and only wrote its
record on the first request — a perfect "holds nothing" ending mistaken for a
missing server. Fix (tooling, honest by construction): the loopback recorder
writes its record AT BOOT (all-zero record = "served nothing yet"; smoke-tested
locally: boot record, HEAD 404 pre-PUT, PUT updates it); `read_record` stays
strict. Chain: #70 abort (#39) → #71 frozen row (#40) → #72 row click → #73
wrong row → #74 record-at-boot — after this every step of both legs has
executed at least once.

## 4. Backlog / next (ordered — milestone [M3: Real transports](https://github.com/kahlanhamza/IUM/milestone/2))

1. ~~#6 — sigv4 signer~~ ✅ (2026-11-02).
2. ~~#7 — S3 multipart adapter core~~ ✅ (2026-11-02, ADR-010 accepted).
3. ~~#8 — planner consumes `max_chunk_bytes`~~ ✅ (2026-11-02, ADR-011 accepted).
4. ~~#9 — S3 CI harness 3-OS + kill-9 drill + >100 MiB nightly e2e~~ ✅ (2026-11-02,
   SeaweedFS 4.51 pinned; MinIO premise dead, Garage Linux-only).
5. ~~#10 — TUS 1.0.0 adapter~~ ✅ (2026-11-02, tusd v2.10.1 pinned, 3-OS CI).
6. ~~**#12 — implement Action::Reattach**~~ ✅ (2026-11-02, required trait method + manager
   mid-transfer & Finishing-entry replay).
7. ~~**#11 — TUS subprocess kill-9 nightly drill**~~ ✅ (2026-11-02, kill-after-ack-
   before-journal window; nightly `tus_drill` job).
8. ~~**M4 tranche decision**~~ ✅ resolved 2026-11-02: "Go for M4" — app shell tranche;
   milestone 3 filed with #13–#16 (see §3).
9. ~~**#13 — `ProbeReport.max_parallel` (TUS ⇒ 1)**~~ ✅ (2026-11-02, ADR-012 accepted).
10. ~~**#14 — unfreeze the shell: ium-app builds green on 3 OS (ADR-006 amendment)**~~ ✅
   (2026-11-02, workspace member + 3-OS CI job + gate.sh app lane).
11. ~~**#15 — runtime wiring: the shell actually uploads (Supervisor, job_updated,
    Canceled state)**~~ ✅ (2026-11-02, engine-side supervisor + honest cancel;
    parse_state corruption fallback removed).
12. ~~**#16 — Sprint-1 UI slice: jobs table, live progress, badges, per-chunk inspector**~~ ✅
   (2026-11-02, pure logic.ts + vitest 9/9 + dumb components + dialog plugin).
13. ~~**M4 milestone complete**~~ ✅ (2026-11-02, all four issues closed, CI 7/7 green).
14. ~~**#17 — real-data e2e drill: a real video through HTTP + TUS + S3**~~ ✅ (2026-11-02,
   M5; ffmpeg fixture, ffprobe + entropy guards on source AND received, byte-exact on
   all 3 lanes, `--with-real` harness flag, CI wired).
15. ~~**#18 — real slow-connection drill: throttled socket, live eviction**~~ ✅
   (2026-11-02, M5; S3-shaped throttled loopback server, `chunk_evicted` with real
   bps 0 vs median 1.74 MB/s, Done in 6.2 s vs ~80 s wait-out, video intact).
16. ~~**#19 — API freeze: public surface reviewed, missing_docs enforced, cargo doc clean**~~
   ✅ (2026-11-02, M5; RED 166 undocumented items → all documented; `sigv4`/`ids` private,
   `below_floor`/`median`/`ThroughputWatch` pub(crate); `#![deny(missing_docs)]`; doc lane
   in gate.sh + ci.yml with RUSTDOCFLAGS="-D warnings").
17. ~~**#20 — semver versioning policy + CHANGELOG.md discipline**~~ ✅ (2026-11-02,
   M5; CHANGELOG.md Keep-a-Changelog seeded from the verified record (0.1.0 = the
   frozen facade), versioning policy in docs/05 (0.x rules, trybuild-pinned breaks,
   changelog-in-same-commit rule), both issue templates carry the changelog checkbox).
18. ~~**M5 milestone complete**~~ ✅ (2026-11-02, all four issues closed, CI 7/7 green).
19. ~~**#22 — real-world stress drill: a real 308 MiB film through S1–S5 vs real SeaweedFS**~~
    ✅ (2026-11-03; free-cloud bench `stress.yml` dispatchable with `victim_rate`/
    `victim_seconds` KB/s inputs; 3 cloud runs — 2 honest failures caught #26 + the
    S2 fast-link kill shape; final run ALL GREEN: S1 308 MiB byte-exact 3.1 s,
    S2 kill at 25% zero-re-upload, S3 32 KiB/s victim evicted → Done 20.1 s vs
    234 s wait-out, S4 pause 0.027 s latency + cancel → server HEAD 404, S5 fleet
    byte-exact; harness fixes: throttle-proxy deadlock, 20 MiB failure-asset floor,
    stdin evidence extractor, deterministic `--allow-calls 1` kill shape).
20. ~~**#24 — verify-repair sweep in the finish gate**~~ ✅ (2026-11-03, caught by S2
    live: resumed upload honestly failed verify on a duplicate server entry → ONE
    counted journaled repair; pinned by `tests/verify_repair.rs`).
21. ~~**#25 — fleet votes keep the eviction floor alive**~~ ✅ (2026-11-03, caught by
    S3 live: lone impaired link after fast parts finish went unevaluated; pinned by
    `tests/lone_worker_eviction.rs`).
22. ~~**#26 — Cancel honored during the Finishing verify gate**~~ ✅ (2026-11-03,
    caught by the first cloud run: parallel parts confirm in one step on fast links
    so the cancel landed on Finishing → refused; `Finishing → Canceled` legal, gate
    honors cancel at entry before any Complete I/O, razor window journals
    `complete_after_cancel`, never a false Done; pinned by
    `tests/finishing_cancel.rs`, RED at the pre-fix HEAD).
23. ~~**#27 — adaptive resplit: pattern #9 driven from measured throughput**~~ ✅
    (2026-11-03, owner question "split chunks again and again for ultimate speed?"
    answered honestly: the resplit machinery existed, fuzz-tested, with ZERO
    runtime callers — and its split loop silently stayed whole at the floor
    regime. Now shrink-only `ResplitPolicy` on the window-close path (median
    bps → target-or-None, 90 s slow-confirm, 25% deadband, would-actually-split
    gate), single-flight winner, epoch watch cancels peers' in-flight PUTs
    BEFORE the reshape, transactional `replace_chunks` + `chunk_resplit` event,
    budgets restart, stale confirms/errors journaled
    (`stale_confirm_dropped`/`stale_error_dropped`) — never applied to a
    stranger chunk. Grow direction drafted and deliberately rejected (S3 parts
    confirm atomically: bigger parts = worse feedback + abandoned prefixes).
    Physics in docs: splitting never creates bandwidth — the win is
    first-confirmation latency, sooner eviction votes, less wire re-sent on
    mid-part death. Pinned by `tests/adaptive_resplit.rs` (RED E0432/E0599;
    GREEN: real ~22 MiB film, uniform 2 MiB/s lane, 1 part → floor-respecting
    pieces, one abandoned prefix, sha256-exact) + the floor-regime pool test +
    policy unit tests; commit `830a080`, gate + CI green).
24. ~~**#28 — worse-case link ladder: owner uplink is B/s-class; the bench SHOWS resplit fire**~~ ✅ (2026-11-03, closed on canonical cloud run 41578950703: S1–S5 + uniform ladder ALL GREEN in one job — 32768 B/s via the MEASURED path (median 72,090 B/s → floor-clamped target), 2048 and 512 B/s via the **FROZEN rule** (`median_bps: 0.0` recorded honestly — nothing measurable, the floor is the only honest target; `ResplitPolicy::plan_frozen`, JOB-level starvation clock, hysteresis holds, unit-pinned + `tests/frozen_link_resplit.rs`). En route: `throttle_proxy --victim-part all` (uniform whole-uplink model) with ≤1 s smooth relay granularity — the 64 KB bursts left ~32 s of upstream silence and the volume server's 30 s idle timeout killed every slow PUT (the B/s-class killer, found via the widened artifact); `VICTIM-TICK`/`VICTIM-FINAL` relayed-byte counter (a canceled engine's FIN queues behind ~1 MB of unread paced data — teardown metering would wait an hour); `realworld_stress s6worse` (fleet-1, timeboxed, journal-truth assertions incl. the self-diagnosing RED error); `stress.yml` `uniform_rates` CSV input; s6worse progress telemetry. Commits `be20179` `000b05c` `cb1b5a6` `1022bb9`; offline 146/146; full local S1–S6 green throughout).
25. ~~**#29 — the user-action scenarios race fast links on CI runners**~~ ✅ (2026-11-03, THREE cloud REDs, all fixed deterministically: S4cancel (41561114324) → in-process `WedgeAdapter` (`e30cb3b`); S4pause (41569827742) → bounded attempt; S4pause again THROUGH a 2 MiB/s leg throttle (41575158721) → the throttle was the wrong tool twice, removed — the **bounded wedge hold**: parts 2–4 hold at entry 10 s, the frontier plateaus at 25% for ten seconds so the poll cannot miss it, and the release creates the lease boundary the cooperative Paused needs to journal (`pause_latency_s: 10.04`, `c80ab42`). `run_bin` captures stderr; s4cancel keeps the unbounded wedge — cancel interrupts, pause cooperates).
26. ~~**Next tranche per docs/04**: more transports (WebDAV/Drive), accounts/OAuth,
    or the Sprint-2 UI — owner decides~~ → superseded 2026-11-03: the owner's readiness
    doubt ("does our engine ready? I don't think so") + the boundary register's prediction
    table (docs/08 §5) filed **M6: Production readiness — the probe generation**
    (milestone 5) FIRST. Owner's feature tranche (WebDAV/Drive, OAuth, Sprint-2 UI)
    decides AFTER M6 — the readiness probes are the next four:
27. ~~**#32 — real-network impairment probe** (loss %, latency/jitter, mid-body RST)~~ ✅
    (2026-11-03, proxy gains `--loss`/`--latency-ms`/`--jitter-ms`/`--reset-after-bytes` +
    `--victim-part none`; `s7net` scenario asserts journal truth per model; harness S7 legs
    + `stress.yml` `net_models`; local AND cloud run 41654248875 green; the probe's first
    run caught the harness's own contamination — victim model armed on the rtt leg —
    `--victim-part none` documents the trap; register rows updated same-commit).
28. ~~**#33 — server diversity probe**: third S3 implementation (`rclone serve s3`, pinned,
    free in CI) + second TUS lane if pinnable; every wire-fact delta pinned or honestly
    recorded (register rows: adapter ↔ server wire ×2)~~ ✅ (2026-11-03, rclone 1.75.2
    lane 23/23 — first run was the RED that caught #36, a latent real-AWS Content-Length
    defect SeaweedFS masked, fixed same session; @tus/server 2.4.7 lane 12/12 with ONE
    loud recorded delta (tusd's GET-download is an extension, not core); wire-fact
    ledgers on both issues; ci.yml Linux diversity legs; docs/08 rows same-commit).
29. ~~**#34 — long-horizon & resource probe**: overnight B/s leg, journal/RSS growth bounds,
    ENOSPC-while-journaling drill — what happens when TRUTH cannot be written (register
    rows: journal ↔ everything)~~ ✅ (2026-11-03 — atomic `set_job_state_event` (both-or-neither
    transitions), honest park Paused + `journal_write_failed` on mid-transfer journal failure
    (best-effort when the disk is still full — loud error, truthful row), real-OS EFBIG pin
    via RLIMIT_FSIZE, soak bounds (16,391 events → 2.4 MB journal / WAL→0 / RSS +5.5 MiB),
    `--with-longhaul` leg live vs SeaweedFS: telemetry + artifact + timebox-pause at a lease
    boundary + fresh-journal replay to Done byte-exact; docs/08 rows same-commit).
30. ~~**#35 — real-shell e2e probe**: tauri-driver on the ubuntu lane drives the actual UI
    through create → progress → pause/resume → Done → cancel/no-object~~ ✅ (2026-11-04,
    BOTH legs green on CI run #75 through the real shell; issue closed with the full
    seam-shakedown ledger; M6 COMPLETE — see docs/04).
31. **Next tranche (owner decides the order)** — the readiness probes are done; the four
    candidate directions from the M6 verdict, each filed as its own issue when started:
    a. **Sprint-2 UX** — the shell is proven end-to-end but the UI is Sprint-1 dumb
       components: drag-and-drop sources, notifications, settings persistence, queue
       management, speed-cap control, per-job detail polish, installer assets.
    b. **More transports** — WebDAV adapter first (plain HTTP family, resumability
       honest-by-capability), then Google Drive (resumable upload protocol). Each new
       transport is a new adapter↔wire probe class per the boundary register.
    c. **Accounts / OAuth layer** — token storage via OS keyring (tauri plugin), account
       model in the shell, adapter auth injection — design note FIRST (touches the
       adapter trait's endpoint/token seam).
    d. **Release engineering toward v0.1.0** — tauri bundle signed installers (3 OS),
       auto-update channel decision, CHANGELOG 0.1.0 cut per the semver-0.x policy.
32. **Kaggle cloud bench** (scaffolding committed: `scripts/kaggle/`, `scripts/kaggle_push.sh`,
    token in gitignored `.secrets/kaggle_token`): move the owner-mandated intensive runs
    off the sandbox — the 17-h overnight longhaul leg and expanded fuzz rounds run there
    as notebook/kernel jobs; results land as artifacts + a session-log row.
33. **Standing hygiene**: rotate the GitHub PAT (pasted into chat 6×, per §10) and the
    Kaggle key at the next convenient break; keep `.secrets/` restore discipline on recycle.

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
   ├─ adapter_tus.rs   TusAdapter — TUS 1.0.0 PATCH/HEAD, 409-realign drift absorption, read-only verify
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
| `docs/08_Boundary_Register.md` | Entity/edge map + seam table: what breaks what, pinned by which test (#31) |
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
| 2026-11-02 | Issue #9 via the full loop (design note → server re-decision → LIVE RED 18/20 `InvalidPart` → `xml_text` single-pass entity decode → harness + drill + CI/nightly wiring → 104/104 + LIVE 23/23 + drill); SeaweedFS 4.51 pinned (Garage Linux-only); wire shapes of BOTH servers pinned as vectors | 3-OS S3 gate live; kill-9 drill: SIGKILL mid-body → zero re-upload → verify→Done | `3ac5dd8` |
| 2026-11-02 | Workflow verification pass, two real catches: (1) `name: >100 MiB …` — unquoted `>` is a YAML block-scalar indicator, the push run died at startup with ZERO jobs (run named after the file path); quoted + validated with `yaml.safe_load`. (2) Windows: the 23-test suite passed LIVE but git-bash `kill` returns before weed.exe releases data-file handles → `rm -rf` hit "Device or resource busy" → `set -e` turned green red; trap now polls until the process dies, then best-effort rm | CI 3-OS green @ `9ae4be6` incl. Windows; nightly `s3_drill` dispatch-verified | `c4ada0c`, `9ae4be6` |
| 2026-11-02 | **Issue #10 closed**: TUS 1.0.0 adapter — byte-level resume. Protocol rehearsed LIVE on tusd v2.10.1 before any Rust (204 + Upload-Offset acks, 409 offset conflict, 404 bogus session, 412 without Tus-Resumable, NO checksum extension offered). `adapter_tus`: probe = POST creation (Location resolved RFC 3986 §5.2, absolute + relative); PATCH `[resume_from, end)` streamed from disk; `bytes_confirmed` = server-acked delta ONLY; **409 realign** absorbs drifted-but-server-acked bytes (HEAD → re-PATCH from server offset → journal converges to server truth; behind-journal = Unresumable, beyond-chunk = Unknown, one attempt only); verify is READ-ONLY HEAD (no commit gate — contrast ADR-010); empty `[(0,0)]` chunk confirms via HEAD. `PatchFailure::Conflict` exposes 409 as data, no string matching. Harness `scripts/tus-test.sh` (pinned tusd, sha256-verified, random port, temp upload dir); ci.yml TUS step on all 3 OS. Two test bugs caught by the server itself (cannot PATCH ahead of server offset; partial-confirm needs real server state) + one telemetry finding pinned: a call containing a failed 409 attempt may buffer ~24 KiB before the rejection — that test asserts the wire floor, the no-failed-attempt drill keeps exact bytes. Loop finding → #12: `Action::Reattach` promised by the taxonomy but fails the job; Finishing-entry verify has an empty session map (pre-existing, S3 included; RED captured on #12) | 115/115 gates; 11/11 LIVE vs tusd incl. 409-realign + byte-level restart drill; #10 auto-closed by commit |
| 2026-11-02 | Issues #11 + #12 filed from loop findings: #11 = TUS subprocess kill-9 nightly drill (the s3_drill shape; blocked by #10); #12 = implement Action::Reattach (mid-transfer SessionLost recovery + Finishing-entry session replay; scratch-test RED captured: job Failed where taxonomy promises recovery) | findings are issues, not drive-bys | — |
| 2026-11-02 | Issue #12 via the full loop (design note → 5×RED — the issue's scratch test + the bounded-recovery count + TWO pre-existing crash drills exposed by the honest session-gated mock → required `reattach` trait method + manager mid-transfer/Finishing-entry journal replay → 118/118); trybuild snapshot regenerated | taxonomy's Reattach promise kept; session truth reconciled from the journal before verify | `96efe6a` |
| 2026-11-02 | Issue #11 via the full loop (drill child with an 8 s ack-hold window → parent asserts the window from the journal itself → resume proves #12 reattach + #10 realign in one fresh process → LIVE GREEN vs tusd 2.10.1 first attempt → nightly `tus_drill` job) | byte-level zero re-upload pinned; kill-after-ack-before-journal provably hit | `fc0d6d6` |
| 2026-11-02 | Session bootstrap: sandbox overlay disk-full (ENOSPC on **mkdir only** — file writes still worked) made `journal::tests::migrations_are_versioned_and_reopen_safe` fail at HEAD; environment defect, not code. Freed ~1.4G (`target/debug/incremental` 505M + `rea-lab/binaries` 933M vendor installers — scan inputs, re-downloadable; evidence `rea-lab/out/` kept per ADR-009). Suite re-verified 119/119 | tests green again at `0ddaaf0`; vendor binaries evicted, re-downloadable via scan scripts | — |
| 2026-11-02 | **M4 milestone filed** ("Go for M4"): milestone 3 + issues #13–#16 with design-note bodies via `scripts/file_m4.py` (idempotent, GitHub REST) | #13 max_parallel (TUS sequential prerequisite), #14 unfreeze, #15 wiring, #16 UI | — |
| 2026-11-02 | Issue #13 via the full loop (design note in body → offline RED `left: 2, right: 1` with Notify-wedged chunk 0 + in-flight high-water mock → honest live finding: tusd's per-upload lock chains ascending chunks, server mercy not protocol honesty, recorded on the issue → manager consumes the journaled declaration each drive round → ADR-012 → 121/121 + LIVE 12/12) | TUS can no longer be driven concurrently against its own declaration | `e333dc5` |
| 2026-11-02 | **First commit of the #13 session included two past-session strays by `git add -A`** (`tools/jdk21` tree, vendor `IDMan.exe` binary) — caught at `git show --stat` BEFORE push, reset, re-committed clean; `.gitignore` now excludes `/tools/` + `rea-lab/**/*.exe` (ADR-009: string dumps are evidence, binaries are not) | repo history never carried the strays | — |
| 2026-11-02 | Issue #14 via the full loop (RED: shell never compiled — no webkit/gtk dev libs and no root; design: CI owns the shell lane natively on 3 OS, local gate runs npm lane unconditionally + loud capability-gated skip; workspace member; icons by script; lockfile committed; ADR-006 amended; roadmap M4 in progress) | gate GREEN (frontend real + loud skip); CI `ium-app` job verifies the shell on every push | `9b215c9`→`6cf1381` (fmt drift caught by the gate, amended) |
| 2026-11-02 | CI verification for #14: the first-ever shell compile ran GREEN on ubuntu + macos + windows (npm lane + clippy `-D warnings` + tests); gate.sh fmt drift (`resume_signal` chain) was caught by the committed-tree gate and amended before push | 3-OS `ium-app` job green at `6cf1381` | — |
| 2026-11-02 | Issue #15 via the full loop (compile-fail RED posted → Supervisor + Canceled state + transition_or_canceled → tests earned a REAL catch: `parse_state`'s `_ => Created` silently resurrected unknown-state jobs as fresh — canceled jobs would come back new; now an exhaustive parse failing loudly → shell wiring (Supervisor-backed Engine, cancel_job/inspect_chunks/stat_file, job_updated watcher) → 126/126) | the shell can finally upload, pause, resume, cancel — all journal-truth | `38d7ead` |
| 2026-11-02 | Issue #16 via the full loop (vitest RED: `./logic` unresolvable → pure logic.ts (badges, engine-mirrored action rules, journal-snapshot math) + dumb components (CreateJobForm via dialog plugin, JobsTable, InspectorDrawer) + list_adapters command → 9/9 vitest + tsc strict + vite build; vitest added to gate + CI) | M4 complete: the app slice renders journal truth with engine-gated actions; UI logic tested locally for real | `b828d9e` |
| 2026-11-02 | **CI shell-lane catches on `b828d9e`** (all 3 OS, frontend+vitest already green): E0433 tokio missing from ium-app deps (watcher sleeps), E0308 `Engine(Arc<Supervisor>)` wrap, unused import under `-D warnings` — the sandbox structurally cannot compile the Tauri crate, so CI is the lane's compiler, and it did its job; fixed forward in `7291606`, **CI GREEN all 7 jobs** (core + ium-app × 3 OS + fmt). M4 milestone closed (4/4 issues); M5 (API freeze & docs) is next per docs/04 | M4 COMPLETE — engine 126/126, shell compiles natively everywhere, UI logic tested | `7291606` |
| 2026-11-02 | **M5 filed** (milestone 4, `scripts/file_m5.py`): #17 real-data e2e drill (owner mandate: a REAL video through all three transports), #18 real slow-connection drill (throttled socket, live eviction), #19 API freeze (missing_docs + cargo doc), #20 semver + CHANGELOG. Roadmap M5 amended; docs/05 gains the standing **"Real data only"** rule | M5 backlog live on GitHub; mandate institutionalized | — |
| 2026-11-02 | **Issue #20 closed; M5 complete**: `CHANGELOG.md` (Keep a Changelog) seeded from the verified record — `[Unreleased]` holds the M5 additions (#17–#20), `[0.1.0] — 2026-11-02` is the first frozen facade with M1–M4 entries all citing issue numbers; versioning policy section added to docs/05 (semver 0.x rules: MINOR may break, PATCH fixes-only, breaking change requires a compile-fail contract test in the bump commit, changelog edit in the same commit, 1.0 gate defined); feature.yml + bug.yml issue templates gained the changelog checkbox (YAML safe_load-validated); workspace version 0.1.0 unchanged and recorded | M5 milestone complete: all boxes in docs/04 checked | see git log |
| 2026-11-02 | **Issue #19 closed** via the full loop (design note → RED: `RUSTFLAGS="-D missing_docs" cargo check` = 166 undocumented public items → surface review → docs → enforcement). Shrink: `pub mod sigv4` (22 items) and `pub mod ids` → private modules (zero external users, verified by grep over app + tests); `below_floor`/`median`/`ThroughputWatch` → `pub(crate)` (only `EvictionPolicy` stays on the facade); net ~26 items off the public API. Everything remaining documented with real guarantees + ADR/invariant links (crate-level docs rewritten: 5 invariants, facade map). Enforcement: `#![deny(missing_docs)]` in lib.rs (clippy lane fails on any new undocumented item), `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` added to gate.sh and ci.yml on all 3 OS; YAML safe_load-validated. 2 rustdoc link warnings fixed (private MAX_WORKERS link, stale `plan` link). Docs + visibility only — zero behavior change | 130/130 + doc lane green; #19 auto-closed by commit | see git log |
| 2026-11-02 | **Issue #18 closed** via the full loop (design note → first run RED ×2 → fix → LIVE green stable 3/3). The drill: local S3-shaped loopback server paces its socket READS (part 2 first PUT at 96 KiB/s, others 2 MiB/s — real TCP backpressure, production S3MultipartAdapter + real SigV4, no test clock); real video fixture from #17; 3-worker fleet; `EvictionPolicy` window 500 ms (documented test shrink), ratios/strikes spec-default. RED earned: (1) drill server's first draft didn't drain the Complete body → keep-alive desync → verify HEAD read XML as request line → ADR-010 verify correctly REFUSED Done (the gate worked; the server was wrong); (2) the abandoned impaired request produces NO FIN inside the window (hyper lingers the pooled socket) — replaced EOF-truncation assertion with the honest observable: impaired attempt still in-flight (started, never completed) at Done. LIVE: `chunk_evicted {bps:0.0, median_bps:1736409, strikes:2, idx:1}` — the watch measured the buffered-burst stall; Done in 6.2 s (wait-out would be ~80 s, bound 30 s); part 2 completed once from the retry (`etag-2-a2`); assembled object sha256 == source, ffprobe-playable. Telemetry finding recorded: eviction fires on the progress freeze that follows hyper's initial buffered burst over a throttled link | 130/130 gates; #18 auto-closed by commit | see git log |
| 2026-11-02 | **Issue #17 closed** via the full loop (survey RED: every fixture in the repo uploads zeros → design note on the issue → `tests/real_data.rs` → LIVE first-attempt green on all three lanes). Real fixture: ffmpeg `testsrc2`+sine → **H.264/AAC MP4, 23,089,475 bytes**, generated at test time (never committed, ADR-009); reality guards on source AND received (ffprobe h264+aac+duration, >64 distinct bytes/64 KiB window — a zeros file holds 1); fidelity invariant `sha256(received)==sha256(source)` + received copy ffprobe-verified PLAYABLE. All lanes ENGINE-driven (create_job→run_job→journal Done): HTTP loopback recorder, TUS vs pinned tusd 2.10.1 (max_conns=4 requested — #13 clamp held), S3 multipart vs pinned SeaweedFS 4.51 (parts reassembled, signed GET read-back). Harness: `--with-real` flag on both server scripts; ci.yml passes it (runner images ship ffmpeg; loud skip elsewhere). Test count 126→129 | LIVE: all 3 lanes byte-exact, same sha256 `1a51b80b…`; #17 auto-closed by commit | see git log |
| 2026-11-03 | **Cancel-during-Finishing (#26) + adaptive resplit driven (#27)** — owner question "do we support splitting chunked chunks again and again for ultimate speed even in very slow connection?" answered honestly (splitting never creates bandwidth; the win is feedback latency: first confirmed byte, first eviction vote, less waste when a connection dies mid-part). #26: `Finishing → Canceled` legal, the verify gate honors cancel at entry (cloud run 1 caught it); pinned by `tests/finishing_cancel.rs`. #27: shrink-only `ResplitPolicy` driven from the measured fleet median on every window close (single-flight winner, epoch watch retires peers' in-flight PUTs before the reshape, transactional plan replace + `chunk_resplit` event, retry budgets restart); grow direction drafted and deliberately rejected (S3 parts confirm atomically — bigger parts = worse feedback + abandoned prefixes); pinned by `tests/adaptive_resplit.rs` (real ~22 MiB film, uniform 2 MiB/s lane, 1 part → floor-respecting pieces, sha256-exact). 143/143 offline; commits `93ff6ba` `028daab` `830a080` `5c35dee` `5ca7f99`, STATUS synced `c314027`; issues #26 #27 closed | 143/143 offline, gates + CI green on every commit | `93ff6ba` `028daab` `830a080` `5c35dee` `5ca7f99` `c314027`, issues #26 #27 |
| 2026-11-03 | **Worse-case ladder PROVEN IN THE FREE CLOUD (#28 closed; #29 three RED→GREEN cycles)** — canonical run 41578950703 @ `1022bb9`: S1 308 MiB byte-exact; S2 zero re-upload; S3 victim 32 KiB/s evicted (30.1 s vs 234 s wait-out); S4cancel 0.079 s deterministic (HEAD 404); **S6 ladder: 32768 B/s via the measured path (median 72,090 B/s → 5 MiB floor), 2048 + 512 B/s via the FROZEN rule (`median_bps: 0.0` — nothing measurable, floor-only target, hysteresis holds)** — "the resplit must happen anyway" delivered: the policy is armed on every connection, fires on measured coarseness, and fires on unmeasurable starvation past slow_confirm with the JOB-level clock (hostile servers reset per-lease clocks with their own 30 s read deadlines). En route, root-caused via widened artifacts + progress telemetry: the proxy's 64 KB bursts left ~32 s of upstream silence → SeaweedFS volume idle timeout (30 s) killed every slow PUT → smooth ≤1 s relay granularity (pre bytes dripped); the runner's tiny buffers made the adapter's first write never complete → median unmeasurable → plan_frozen. Deterministic user-action scenarios: S4cancel unbounded wedge; S4pause BOUNDED wedge hold (10 s plateau at 25% → `pause_latency_s: 10.04`); the 2 MiB/s leg throttle was the wrong tool twice and is gone. Offline 146/146; full local S1–S6 green at every step; gate discipline slip (doc lane failed through a pipe-masked exit) caught and fixed forward in `1022bb9` | evidence on #28/#29; BOTH closed | `be20179` `000b05c` `cb1b5a6` `1022bb9` `c80ab42`, runs 41561114324 / 41569827742 / 41575158721 / 41576795816 / 41577998607 / 41578950703 |
| 2026-11-03 | **CI forensics + engine re-proof on owner request (#21)** — "look to CI history, why failed/cancelled; does the engine really work? upload must be fast even on slow connection". All 40 runs audited: **4 failures, all root-caused and already fixed in-tree** (`99ed179` = defect #11 missing dev-dep, institutionalized as the committed-tree gate; `3ac5dd8` = unquoted `>` YAML block-scalar in a step name, fixed `c4ada0c`; `c4ada0c` = Windows-only cleanup race, 23/23 tests PASSED, `rm` hit weed.exe's async handle release, fixed `9ae4be6`; `38d7ead`+`b828d9e` = shell glue compile errors E0433/E0308 caught by CI's 3-OS ium-app job — the lane CI owns by design, fixed `7291606`). **4 cancellations all benign**: `ci.yml` `concurrency: cancel-in-progress: true`; each superseded by a newer main push within 22 s–2 min, every superseding run green. Fresh local execution at `141d9b5` (nothing trusted from history): 130/130 offline; LIVE vs real servers with the real 23 MB H.264/AAC video — S3 23/23 + byte-exact video (sha256 `1a51b80b…`), TUS 11/11 + byte-exact, S3 kill-9 drill zero-re-upload, TUS kill-9 drill realign-resume; **slow-connection eviction drill**: pool median 2,031,033 B/s, impaired 96 KiB/s link measured ~0 B/s in-window → `chunk_evicted` on live wire numbers, impaired attempt abandoned mid-body, retry at full speed, **Done in 5.7 s vs ~80 s wait-out (~14×)**, video byte-identical + playable | CI healthy (last 7 runs green); verdict + tables recorded on the issue | `141d9b5` audit, issue #21 |
| 2026-11-03 | **Real-world stress campaign GREEN + free-cloud bench proven (#22, + #24 #25 #26 found & fixed)** — owner mandates: real-world stress on a REAL film, a free cloud service if the sandbox disk can't take it, and KB/s-scale honesty ("23.8 MB/s is loopback overhead, not a network; my uplinks are kilobytes per second"). Built & executed: `realworld_stress` scenarios S1–S5 (path-agnostic `--file` — the owner's exact video drops in with zero code change), `throttle_proxy.py` (real TCP choke in front of the REAL server), `scripts/realworld-stress.sh` driver, `stress.yml` **dispatchable free-cloud bench with `victim_rate`/`victim_seconds` inputs** (GitHub Actions = the €0 developer cloud, 14+ GB disk; real 308 MiB 1080p film sha256-pinned per run, never committed). First full local execution caught **three harness defects** (proxy deadlocked every proxied PUT by discarding pipelined body bytes; 10 MiB failure assets plan one chunk so victim part 2 cannot exist → 20 MiB floor; s4pause raced the cooperative-pause contract) — engine clean every time. Cloud run 1: S1 308 MiB byte-exact 2.97 s + S2 + S3 + S4pause GREEN, **S4-cancel failed → #26** (Finishing refused Cancel — real KB/s uplinks spend seconds in verify; fixed: transition legal, gate honors cancel at entry, razor window journals `complete_after_cancel`, never a false Done; pinned RED→GREEN by `tests/finishing_cancel.rs`). Cloud run 2: **S2 racing kills cannot land on fast links** (parallel parts confirm in one step; post-completion kill hit an already-Done journal, resume honestly refused) → deterministic `--allow-calls 1` wedge shape. **Cloud run 3 (41548311010): ALL FIVE SCENARIOS GREEN** — S1 308 MiB byte-exact 3.11 s; S2 kill at exactly 25%, `wire_resent == size − confirmed` (zero re-upload); S3 32 KiB/s victim (≈256 kbps uplink) evicted at 2 strikes vs median 26.2 MB/s → **Done 20.1 s vs 234 s wait-out (~11.6×)**; S4 pause journaled in 0.027 s + wire frozen + cancel → **server HEAD 404 (a canceled upload never completed)**; S5 fleet (308 MiB + 30 MiB, one manager) both byte-exact. Offline suite 135/135 local (incl. new finishing_cancel); gate GREEN on committed tree `5ca7f99`; one gate-discipline slip (pipe masked gate exit → pushed a clippy-red test) fixed forward in `028daab` and the gate is now always run with pipefail | #22 #24 #25 #26 all closed with wire evidence; CHANGELOG entries for #22-harness + #26; weekly keep-alive Saturday 04:00 UTC | `1ad5c8d` `c718a0b` `548cb00` `5c35dee` `93ff6ba` `028daab` `5ca7f99`, issues #22 #24 #25 #26, runs 41545100471 / 41547293634 / 41548311010 |
| 2026-11-03 | **Nightly tus_drill RED → #30: the 64 KiB wire ceiling was a calibrated race — zero-re-upload now EXACT.** Nightly 41618615182 failed `resume moved 21168128 for 20971520 missing — beyond the 64 KiB slack`. Root cause proven, not assumed: the rejected-409 PATCH's socket flush (frames hyper pulls before `send()` resolves) is a client↔server scheduling race — 0–48 KiB across 11 quiet local runs, 192 KiB on a loaded runner; A/B worktrees at pre-#27 (`93ff6ba`) and pre-#28 (`5ca7f99`) show identical distributions, and the journal-capturing `scripts/dbg-tus-drill.sh` (new) shows the resume event log is exactly `SessionLost → Reattach → realign confirm` — manager fully exonerated. Protocol rehearsal (project rule): tusd consumes NOTHING of a rejected body (offset immutable across ~12 MiB of rejected probes); `Expect: 100-continue` REJECTED by rehearsal — reqwest 0.12/hyper 1.12 neither holds the body on conflict (8192 pulled = control) nor survives the happy path (`BodyWriteAborted, NotEof`) — negative result recorded on the issue. Fix `fd44908`: `ProgressReporter::waste()/wasted()` measures the flush (moved() semantics untouched — eviction/resplit keep counting real wire bytes); `TusAdapter::upload_chunk` reclassifies at the Conflict branch; the drill child prints `WASTED` and the parent asserts `WIRE − WASTED == missing` EXACTLY (re-sent server-held bytes are not waste-reclassifiable, so a real re-upload still fails loudly); offline realign test gained `moved() − wasted() == tail` (RED: E0599 pre-fix; runtime RED: WASTED-line missing vs pinned tusd). GREEN: 146/146 offline, live suite 12/12, drill exact on 5 hand-driven runs (one hit flush = exactly 65536 — the old ceiling's luck, quantified); gate GREEN on `fd44908` (first gate attempt died `ld` signal 7 Bus error — sandbox disk at 99% from the A/B worktrees; cleaned, re-run clean, not a code failure); nightly judge 41643285429 all 4 jobs GREEN incl. tus_drill; CHANGELOG #30 + late #28/#29 entries | issue #30 closed with evidence | `fd44908`, run 41643285429 |
| 2026-11-03 | **Boundary register institutionalized (#31)** — the owner's "we must understand the relationship between entities and know what gonna break" answered as a checkable repo artifact, not chat prose: `docs/08` (forensic verdict over #23–#30 incl. the #30 A/B exoneration — zero regressions from fixes, all latent edge defects; entity map re-derived from a fresh `use crate::` grep at `812188f`; seam table mapping EVERY defect to date to its edge with today's pinning test; the 3-field design-note rule; the probe-class prediction table that motivated M6); docs/05 step 2 + Rules of engagement amended; feature/bug templates carry the register field (yaml.safe_load-validated). Docs + templates only, zero engine code | committed-tree gate green on `9b8e49b`; #31 closed by commit | `9b8e49b` |
| 2026-11-03 | **M6 filed — production-readiness probe generation** (milestone 5, issues #32–#35 via `scripts/file_m6.py`): the owner's "does our engine ready? I don't think so" answered honestly — the engine is hardened on TWO pinned servers, ONE impairment axis (bandwidth), minute-scale horizons, and a never-driven shell. The four probes come straight from docs/08 §5's prediction table (a new probe class = first-time seam exposure): #32 loss/latency/RST (eviction+retry seams), #33 server diversity (adapter↔wire seams), #34 long-horizon+ENOSPC (journal seam), #35 real-shell WebDriver e2e (shell↔supervisor seam, never driven once). docs/04 M6 section added; STATUS backlog rewritten; feature tranche decision (WebDAV/Drive, OAuth, Sprint-2 UI) deferred to the owner until after M6 | milestone 5 open 0/4; probes ready for the loop | `f8e0208` |
| 2026-11-03 | **#32 CLOSED — real-network models through the full loop** (design note + RED on the issue → implement → local real-data GREEN → cloud GREEN): `throttle_proxy.py` gains `--loss PCT` (RTO-scale stalls U(0.2,2.0)s per relayed segment — the application-observable of packet loss; a userspace relay CANNOT drop in-stream bytes), `--latency-ms/--jitter-ms` (connect+response RTT, bandwidth untouched), `--reset-after-bytes N` (ONE-SHOT hard RST of both ends via SO_LINGER=0 when the first UploadPart PUT crosses N bytes), `--victim-part none`; all sleeps responsive (≤250 ms steps watching the client — the #28 lesson). `realworld_stress s7net --model loss\|rtt\|reset` asserts journal truth per model (rtt: ZERO evictions/resplits — an alive-but-latent link is never churned; reset: ONE counted `chunk_error {kind: Transient, action: Retry}` then Done); harness S7 legs carry wire truth (LOSS/RESET-WIRE lines) + server truth (read-back sha256); `stress.yml` `net_models` input; docs/08 rows updated same-commit. **The probe caught its first bug on its first local run** — the rtt leg's own assert refused to pass because the harness had left the victim model ARMED (`--victim-part 2` = 96 KiB/s default); the engine's eviction of that secretly-throttled part was textbook-correct — fixed with `--victim-part none` + help-text trap warning. Local: S1–S5 + all three S7 legs green (real 30 MB film `5b603ec1…`, SeaweedFS 4.51: loss = 9 stalls absorbed 0 evictions; rtt = 16 pauses 0 churn; reset = RST at 1,064,960 B exactly once + one counted retry). **Cloud run 41654248875 @ `6f514c7` ALL GREEN**: loss = 11 stalls/0 evictions @ 6.3 s; rtt = 16 pauses/0 churn @ 4.5 s; reset = RST at 1,097,728 B exactly once, retry completed byte-exact @ 0.54 s. Offline 146/146; committed-tree gate GREEN | issue #32 closed with wire evidence | `6f514c7`, run 41654248875 |
| 2026-11-03 | **#33 closed — server diversity probe + #36 (adapter defect it caught)**: `s3-test.sh --server rclone` (pinned rclone 1.75.2 `serve s3`, SHA256SUMS supply-chain guard) — first-ever third-implementation run was RED 22/23: zero-byte PutObject went out with NEITHER Content-Length nor chunked-TE and gofakes3 enforces AWS-real 411 MissingContentLength where SeaweedFS was lenient (latent real-AWS bug) → #36 filed with the RED, fixed same session (explicit `content-length: 0`, UploadPart's existing mechanism) → rclone 23/23 + SeaweedFS 23/23. `tus-test.sh --server tus-node` (pinned @tus/server 2.4.7 + @tus/file-store 2.1.2, committed package-lock.json = integrity guard) — 12/12 with ONE loud recorded delta: tusd's GET-download is an extension, NOT tus core (read-back tests loud-skip via `IUM_TUS_SERVER`). Wire-fact ledgers on both issues (ListParts-works-despite-docs, `&#34;` numeric entities re-confirmed, 409/412/204-Location facts); ci.yml Linux diversity legs; docs/08 rows updated same-commit | rclone 23/23, tus-node 12/12, canonical lanes re-green, offline 146/146, committed-tree gate GREEN on `a35d881` |
| 2026-11-03 | **#34 closed — long-horizon & resource honesty**: the journal seam probed and hardened. **RED ×2 at HEAD** (posted on the issue): (1) mid-transfer ENOSPC left the job dangling `Running` (loud to the caller, a lie of omission in the journal), (2) the HALF-TRANSITION — `transition()`'s two autocommit writes let a disk-full commit the state row and lose its event. Fix: `SessionJournal::set_job_state_event` (UPDATE + event in ONE transaction — both-or-neither) + `JobManager::park_on_journal_failure` (on `EngineError::Journal` escaping drive/finish: best-effort atomic park to `Paused` + `journal_write_failed`; when the disk is STILL full the park itself fails loudly, the row stays truthful, the original error always returned; `Finishing` intentionally stays Finishing — resume re-enters the verify gate, Complete idempotent per #24). Drill hooks: `SessionJournal::drill_fail_writes` (SQLite-test-suite-style injected SQLITE_FULL) + `tests/enospc_rlimit_drill.rs` (REAL EFBIG: RLIMIT_FSIZE + SIGXFSZ-ignored in its own process — pins loud + truthful + recoverable + byte-exact-after-free). Soak: `tests/soak_bounds.rs` drives the REAL manager through 16,391 confirm events (64 MiB in 4 KiB server-acked segments) — **measured 2,502,656 B journal after `checkpoint_wal(TRUNCATE)` (≈153 B/event, linear), WAL → 0, RSS delta 5,644 KiB** — bounds pinned at 32 MB / 4096 B / 256 MiB. Longhaul: `realworld_stress longhaul` + `scripts/longhaul.sh` + `s3-test.sh --with-longhaul FILE RATE SECS` — uniform paced lane, `[longhaul][persist]` telemetry (frontier / journal bytes / RSS), honest TIMEBOX-PAUSE (never cancel; lands at the next lease boundary — part-scale physics documented), FRESH-journal replay + resume to Done, direct read-back sha256; **live vs SeaweedFS 4.51: pause landed at 176 s, replay resumed byte-exact, artifact in `rea-lab/longhaul/`**; the first live run's own artifact caught the harness bug (endpoint bypassed the proxy — no UNIFORM lines) — fixed, re-run green. docs/08 rows same-commit. 30 MB @ 512 B/s ≈ 17 h overnight config documented on the issue (honest in-session timebox-cancel; the harness is what shipped) | offline 150/150 (fmt + clippy `-D warnings` + tests), ENOSPC drills GREEN (injected ×2 + real-OS ×1), soak within pinned bounds, longhaul GREEN live, #34 closed by commit |
| 2026-11-03 | **#37 closed — CI debt on main cleared (owner: "take a look at CI, failed ones you didn't solve, and Queued ones too")**: audit of all 82 runs → runs #62/#63 red after #33/#34 (#62's macos job = concurrency-cancel artifact, 0 queued/in-progress remained). Two real failures: (A) windows clippy E0422/E0425 — `enospc_rlimit_drill.rs` used Unix-only `libc::rlimit/setrlimit/RLIMIT_FSIZE` outside any cfg gate (test fn was gated, the adapter struct wasn't) → file-level `#![cfg(unix)]` (the drill is Unix BY CONSTRUCTION; windows compiles an empty target). (B) @tus/server diversity lane `tus_bogus_session_maps_to_session_lost` (run #63; #62 same code GREEN → flake): @tus/server early-rejects the PATCH (writes 404, closes WITHOUT consuming the body) so the client streaming 6 MiB sees a reset, not the status → adapter now HEAD-probes after ANY PATCH transport failure and lets the SERVER's answer decide (`404/410` → SessionLost, success/HEAD-failure → Transient — retries stay counted; dead-server negative path re-proven Transient). **Second face found while proving the fix locally (1-in-3 lane RED)**: realign exact-waste assert 8 KiB short (`moved−wasted = 2097152+8192`) — the rejected attempt's last chunk landed in `moved()` AFTER the instant snapshot (counted on hyper's connection task, caller read it same-instant) → waste now classified by SETTLED ARITHMETIC after the realign (`pushed − tail`, tail = acked re-PATCH span — exact whenever the count landed) + `ProgressReporter` counters SeqCst for cross-task coherence. Evidence: `scripts/issue37_red_evidence.md` (issue body), `scripts/issue37_green_evidence.md`; docs/08 #37 wire-fact row same-commit | lanes: tusd 4×12/12, tus-node 6×12/12 (was 1-in-3 RED), SeaweedFS 23/23, rclone 23/23, offline suite 0 failed, fmt+clippy clean | `8950cd8` |
| 2026-11-03 | **#37 follow-up — THIRD face of the reset race, probe made three-way**: run 41682579860 green on windows + bogus-session, but the same early-reject then ate the realign test's **409** (`stale-offset PATCH → reset; HEAD probe said "session exists" → Transient; realign never ran`) → the POST-transport-failure probe now classifies by SERVER TRUTH: HEAD 404/410 → SessionLost; HEAD 200 with offset ≠ attempted → `PatchFailure::Conflict` (the 409 the client never saw → realign dance, flush reckoned waste by the settled arithmetic); else → Transient (retries counted). Rehearsal fourth wire fact: `path: "/files/"` makes @tus/server 2.4.7 emit DOUBLE-SLASH Locations `/files//<id>` on every creation (docs/08 row). Local: tus-node 8×12/12 consecutive, tusd green, offline green | issue #37 reopened + re-closed by the follow-up commit | follow-up commit this push |
| 2026-11-04 | **#37 reopened by the owner and this time faced in full — "first deal with failed CI from #62 to #68, stop skipping it"**: sandbox recycle had wiped `.secrets/` again (token restored from the session record, 6th paste logged in §10); git had DIVERGED (local `e6dbe55` = origin `f73d62e` + the unpushed Kaggle scaffolding — reset to origin, Kaggle scripts staged aside uncommitted). **Full audit of runs #62–#69 posted on the issue** (no more pushing while red): #62 macos = concurrency-cancel phantom; #63 = windows rlimit compile (already fixed in-tree) + bogus-session flake (already fixed by the three-way probe); #64 = realign flake (already fixed by settled arithmetic — core green on all 3 OSes × 5 consecutive runs #65–#69); **#65–#69 = app-e2e failed 5× IDENTICALLY at `e2e_shell.rs:434` `live progress must move: first=0% second=0%`** — root-caused as a DESIGN FLAW, not flakiness: leg A never touched the adapter `<select>` → registry default `http_oneshot` → ONE chunk planned → `bytes_confirmed` journals only at PUT completion → the UI's honest Σ-confirmed percent is 0% for the whole ~12 s throttled transfer then 100% → the two-sample assert was unsatisfiable BY CONSTRUCTION (no throttle tuning can fix it). **Fix (product-true, no assertion weakened): leg A drives the tus adapter.** `loopback_recorder` gains a real TUS lane (POST /files creation → relative Location; throttled PATCH → 204 + server-acked Upload-Offset; HEAD truth; 409-after-drain / 412 / 404-under-/files* shapes; record JSON v2 `tus_sessions/tus_bytes/tus_last`) — proven by running the adapter's OWN 12-test wire suite against it: 12/12 (then the whole suite vs @tus/server and tusd too). e2e leg A: `set_select` via W3C execute-sync (WebKitGTK select popup is a native menu unreachable under xvfb; native value setter + real change event — every other interaction stays real input), connections=6 → 5 chunks ≥ 5 MiB floor, live-progress assert = deadline poll (movement within 45 s) instead of two blind samples, server truth = record `tus_bytes` + HEAD session URL `upload-offset == size`; leg B explicitly re-selects `http_oneshot` (cancel-nothing semantics stay one-shot's). **Flake faces stress-buried: `scripts/stress_tus_flakes.sh` 36/36 (12× recorder + 12× @tus/server 2.4.7 + 12× tusd 2.10.1, 0 failures)**. Offline 150/150, fmt + clippy `-D warnings` clean. Full app-e2e stays CI-proven (webkit2gtk-dev/tauri-driver need root, absent in sandbox) | audit comment + evidence on #37; closes when this push's run is green | this commit |
| 2026-11-04 | **Issue #39 — the shell's first-ever Upload click killed it (runs #65–#70 root cause).** Run #70 (leg-A tus rework) got PAST the submit-enable blocker of #65–#69 and was the FIRST CI run in history to click Upload — the app aborted on the spot (`Session terminated without a reply` on the click; driver.log ends in a Rust backtrace + "thread caused non-unwinding panic. aborting.", which `tail -25` had been truncating). Root cause proven, not guessed: tauri v2 runs SYNC commands on the GTK main thread ("Commands without the async keyword are executed on the main thread"); `create_job`/`resume_signal` → `Supervisor::drive` → raw `tokio::spawn` → "no reactor running" panic → unwind crosses the webview IPC FFI boundary → abort. Unit tests structurally couldn't see it (all `#[tokio::test]`); the e2e was built for exactly this class. Loop honored: issue #39 filed with the diagnosis → RED `ium-core/tests/supervisor.rs::drive_is_callable_off_the_runtime_thread` (panic captured at supervisor.rs:77, posted on the issue) → fix: `Supervisor` captures a `tokio::runtime::Handle` at construction, `with_handle()` for context-less hosts, `drive()` spawns via the stored handle (falling back to the ambient context), honest panic only for a construct-context-less-AND-handleless host; second test pins the shell's exact shape (construct AND drive off-runtime → Done); ium-app setup passes tauri's async-runtime tokio handle (read via `block_on` — `async_runtime::handle()` returns tauri's own RuntimeHandle enum, not the tokio handle). Same commit: ci.yml `cancel-in-progress: false` (#62's phantom red can never recur — red must mean RED) + app-e2e.sh forensics `tail -200` (issue #39 lesson). Sandbox disk hit 99% mid-gates (ld signal 7 Bus error) — evicted 1.2G browser caches + 3.4G rebuildable test executables, re-ran clean | 87/87 ium-core suite green (supervisor 6/6 incl. both new), fmt + clippy `-D warnings` clean, RED+diagnosis on #39; #37 + #35's lane ride this push; closes when the run is green | `940ddc9` |
| 2026-11-04 | **Issue #40 — the ACL granted the webview NOTHING (run #71 evidence).** #39's fix proven FIRST TRY on 7/8 jobs (core ×3 OS + ium-app ×3 OS + fmt — the with_handle wiring compiles everywhere); app-e2e then proved the click survives by WIRE TRUTH — recorder `tus_sessions: 1, tus_bytes: 25165824`, the full 24 MiB through the real shell via tus — and caught the next seam: the row froze at `probing pct=0` for the entire 30 s poll while the engine transferred (the job by journal truth went Running→Finishing→Done). Diagnosis: no `capabilities/` file existed; tauri v2's ACL allows APP commands by default (docs quote on the issue — that's why every invoke worked) but core-plugin commands are gated, and the `job_updated` stream is `plugin:event|listen` → silently denied (the listen() promise rejection is unhandled in App.tsx — silent by construction). Same gap broke the dialog-plugin file picker for real users. Fix: `ium-app/src-tauri/capabilities/default.json` = `core:default` + `dialog:default`, window `main` (validated JSON; the identifier set matches the docs' own example capability). CHANGELOG [Unreleased] Fixed entries for #39 + #40 | gates re-run on the committed tree; #40 closes when the e2e lane goes green | this commit |
| 2026-11-04 | **Run #72 → #35 harness fix (row click).** #40's fix proven: leg A fully green through the real shell (recorder full 24 MiB; running → live progress → pause → resume → Done), then `element not interactable` clicking the row for the inspector — the jobs table is wider than the window (7 cols + 3 buttons), a full-width `<tr>` cannot be scrolled into view horizontally (geckodriver#1294 class); harness-only fix: click the row's first CELL (bubbles to the `<tr>` onClick — same user path). Evidence + mechanism on #35 | e2e compiles, fmt + clippy clean; closes when the lane goes green | this commit |
| 2026-11-04 | **RED STREAK ENDED — run #75 ALL GREEN (8/8 jobs), the first since #61; #35 + #37 closed on the owner's raw-log evidence.** The owner pasted run #74's job log directly; it showed the deepest e2e failure mode yet: EVERY assert before it passed (row B found by file, `running`, cancel clicked, `Canceled` badge) and the test died at the last server-truth read — `recorder-b.json never appeared` (10 s poll). Root cause: a canceled PUT dies mid-body, the recorder's handler takes the read error, and the OLD recorder wrote its record file only after a completed request — a perfect "holds nothing" ending was indistinguishable from a missing server. Fix `dd046a7`: the record file is a server-truth surface and now exists from BOOT (all-zero record IS the truth "served nothing yet"). Verified locally before the single push: committed-tree gate GREEN (152/152 core, fmt, clippy `-D warnings`, vitest 9/9) + a wire-level smoke replaying the exact #74 scenario (boot-record exists + HEAD 404 pre-PUT + RST-mid-body leaves zeros + completed PUT still updates the record). One push → run 41726278945 GREEN; #35 and #37 closed with full ledger comments (every red #62–#74 mapped to its fix). Kaggle token restored to `.secrets/kaggle_token` (600, uncommitted) per the standing security note | run 41726278945 8/8 success; issues #35/#37 closed; row authored locally (unpushed by design — no CI run spent on docs) | `dd046a7` |
| 2026-11-04 | **Housekeeping tranche after the green run — README to product standard, PR #38 resolved, roadmap truth.** (1) README.md rewritten from the stale Sprint-0 text ("45 tests", shell "parked") to a scannable product README: badges (ci/nightly/stress), highlights table, architecture diagram, invariants, quick start, transport/honesty table, test-lane matrix with the drills' pinned numbers, repo map, doc index, the loop as Contributing, security, status — every claim traced to the verified record (152 core tests recounted per-suite this session; 5 real server implementations; kill-9 zero-re-upload; soak ≈153 B/event). (2) PR #38 (`e2e-fix` → main, the async-commands variant of the #39 fix) CLOSED as superseded, not merged: main's `940ddc9` fixes the same abort at the root (`Supervisor::with_handle`, every spawn site, +2 regression tests) and run #75 proves the shell end-to-end; rationale comment posted on the PR, branch deleted. (3) docs/04 M6 checkboxes ticked + verdict paragraph (probes closed; #36/#39/#40 found-and-closed chain; next tranche = owner's call). (4) STATUS backlog §4 rewritten: #35 ✅, items 31–33 = the next-steps plan (Sprint-2 UX / WebDAV+Drive transports / OAuth+keyring / v0.1.0 packaging / Kaggle cloud bench for the 17-h longhaul + fuzz / token rotation). (5) Kaggle scaffolding + session evidence scripts committed (secret-scanned first: no token material in any tracked file). Sandbox recycled again mid-task (7th): `.secrets/` + `.toolchain/` wiped and restored per §0/§10; `.gate-target` cleared for disk headroom | gate on committed tree; push = one CI run on docs-only + untracked-script changes | this commit |

## 10. Security

- Token state: fine-grained PAT restored into `.secrets/gh_token` on 2026-11-02 (gitignored,
  chmod 600); re-pasted a FOURTH time and re-stored 2026-11-03; **pasted a FIFTH time and
  re-stored 2026-11-03 (sandbox recycle wiped `.secrets/` again — the file must be
  restored after every recycle). The owner has now pasted it into chat five times — rotate
  it at
  <https://github.com/settings/personal-access-tokens> when convenient.** After rotating:
  `mkdir -p .secrets && chmod 700 .secrets && echo 'NEW_TOKEN' > .secrets/gh_token && chmod 600 .secrets/gh_token`
  (never commit this file; scopes needed: repo contents read/write, issues read/write,
  metadata read).
- **2026-11-04: the recycle wiped `.secrets/` a third time** — token restored from the
  session record (6th restore, same value, still gitignored `chmod 600`; rotation still
  advised). The owner's **Kaggle API token is now stored as `.secrets/kaggle_token`**
  (same gitignore + 600 discipline, never committed, never printed) for the
  free-cloud intensive runs (`scripts/kaggle_push.sh` reads it; `scripts/kaggle/`
  holds the kernel program). Same rule as the PAT: rotate at kaggle.com/settings if
  it ever leaks into chat history beyond the session record.
- `.secrets/` and `.toolchain/` are gitignored; `core.fileMode false` is set (sandbox mode churn).
