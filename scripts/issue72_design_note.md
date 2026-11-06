# Lane 5 (#66): 100-jobs load e2e + installer-size budget gate

Product bar #66, lane 5: **product scalability** — the engine must survive a
100-job queue (bounded, honest, journal-intact) and the installers must stay
small by DECISION, not by accident. Two deliverables, one lane.

## Wire facts (all executed in this sandbox today, none assumed)

- **W1 — installer sizes today are measured but unguarded.** v0.1.1 release
  assets (read from the release page): NSIS `x64-setup.exe` 4.29 MB, MSI
  6.02 MB, DMG (aarch64) 5.97 MB, deb 6.57 MB, rpm 6.57 MB, AppImage 81.2 MB.
  `release.yml` collects into `ium-app/dist_release/` with sha256 sidecars and
  prints the sizes — but NOTHING fails if a dependency bump inflates an
  installer 10×: the #65 bar says "lightweight + fast, MEASURED", measurement
  exists, enforcement does not.
- **W2 — the supervisor spawns UNBOUNDED.** `Supervisor::drive()`
  (ium-core/src/supervisor.rs:103) spawns one `run_job` task per call, no cap.
  The shell constructs the supervisor once (ium-app/src-tauri/src/lib.rs:323,
  `Supervisor::with_handle`) and the intake drives per job: a 100-file
  drag-drop = 100 simultaneous uploads competing for one uplink — every
  mainstream manager bounds this (aria2 ships `--max-concurrent-downloads=5`).
- **W3 — 100 concurrent jobs over one manager is untested.** The soak (#34)
  pins event-scale bounds for ONE long job; the supervisor tests (#15) drive
  single jobs. Nothing drives 100 jobs through one shared `JobManager`
  (Arc-shared; per-job single-driver guard; journal mutex serializes writes).
- **W4 — the failure-path invariant is testable at scale.** RetryPolicy is
  injectable (`set_policy`); an adapter keyed by endpoint can fail transiently
  k times then succeed (→ Done with journaled retries) or always fail
  (→ Failed after the attempt budget). The invariant "every failure path
  terminates in Done or Failed" must hold for 100 interleaved state machines.
- **W5 — the drive contract is spawn-and-walk-away with journal-truth
  polling** (supervisor tests): `sup.drive(id)` → poll `snapshot(id).state`.
  `resume_signal` re-drives through `drive()`. A limit must not break
  pause/resume/cancel semantics, and `is_driving` must stay honest (queued ≠
  driving).

## Design

- **D1 — an OPTIONAL limit, legacy paths untouched.**
  `Supervisor::with_concurrency_limit(manager, max)` — `max ≥ 1` enforced
  honestly (0 panics with a message). `new` / `with_handle` stay UNBOUNDED:
  byte-for-byte today's semantics for every existing caller.
- **D2 — FIFO queue under one lock.** `drive()` under a limit: if
  `tasks.len() < max` → spawn as today; else push onto a FIFO queue (dedup:
  a job already queued is not re-queued). The drive task, on completion
  (whatever `run_job` returned), removes itself and releases the slot: pop
  queued ids, DROPPING entries whose journal state is terminal (canceled or
  failed while queued are never driven), spawn the first live one. `active`
  is derived (`tasks.len()`), not tracked — fewer invariants to lie.
- **D3 — queued jobs keep full semantics.** cancel-while-queued: the manager
  journals Canceled (Created is cancellable), no task to abort, the stale
  queue entry is dropped at pop time. pause-while-queued: the flag is set;
  when the slot frees, run_job sees Paused-before-first-lease and parks the
  job honestly; `resume_signal` re-drives THROUGH the limiter (it calls
  `drive`). `is_driving` stays false for queued jobs — the journal remains
  the only user-visible truth.
- **D4 — the shell boots bounded.** ium-app wiring: `with_concurrency_limit`
  with the same explicit runtime handle (issue #39 lesson kept), limit 4
  concurrent drives (aria2 ships 5 for downloads; uploads are heavier). The
  queue is invisible to the UI: queued jobs read Created; JobsTable already
  renders that badge. Shell compile proof comes from CI (the sandbox has no
  GTK — the standing honesty contract).
- **D5 — the load e2e (`ium-core/tests/load_100.rs`).** ONE manager, ONE
  journal (tempdir), 100 jobs, sizes 256 KiB … 4 MiB, driven via
  `sup.drive()` through a limit of 8, fast RetryPolicy (1 ms backoff).
  Behaviors keyed by endpoint: 88 jobs fail transiently k ∈ 0..2 times then
  succeed → Done; 8 jobs fail forever → Failed; 4 jobs clean → Done.
  Asserts: exact terminal mix (92 Done / 8 Failed, zero stuck); the transient
  jobs' journals prove the retries (attempt events); fd delta bounded;
  RSS delta bounded; wall time bounded; a `LOAD-100 {json}` summary printed
  for the issue record (the soak's honesty pattern).
- **D6 — the budget gate is data + a checker, not vibes.**
  `scripts/installer_budget.json`: per-artifact `max_bytes` +
  `baseline_bytes` (the v0.1.1 measured numbers) + `baseline_source`. Budgets
  (MB): NSIS 10, MSI 12, DMG 12, deb 12, rpm 12, AppImage 110 — honest
  headroom over the baseline, tight enough that bloat is a REVIEWED diff to
  this table, never a silent ship. `scripts/installer_size_gate.py`
  (stdlib-only): scans a dir for installer suffixes (.msi/.exe/.dmg/.deb/
  .AppImage/.rpm), checks each against its budget; ANY installer-suffix file
  without a budget entry FAILS (no silent pass for new formats); table output
  (actual vs budget vs baseline, headroom %, verdict), exit 1 on any
  violation; `--dir`/`--budget` args; `--self-test` builds synthetic fixtures
  and proves all four verdicts (pass / over-budget / unknown-format /
  missing-budget-entry).
- **D7 — the gate runs where the artifacts exist.** `release.yml`: each
  per-OS matrix job runs the gate right after collect (`ium-app/dist_release`)
  — an over-budget installer fails ITS job before upload/publish can ship it.
  `ci.yml`: `--self-test` rides the ubuntu core job — the gate logic stays
  alive between releases, not just at release time.

## RED

- **RED-1** — `tests/load_100.rs` + limiter behavior tests reference
  `Supervisor::with_concurrency_limit` → E0599 at HEAD (function absent).
- **RED-2** — `python3 scripts/installer_size_gate.py --self-test` → the
  script does not exist (exit ≠ 0).

## Gates

`cargo fmt --all -- --check` · `cargo clippy -p ium-core --all-targets -- -D
warnings` · `cargo test -p ium-core` (269 + the new lane) · gate self-test ·
YAML parse of both workflows.
