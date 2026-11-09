# Issue #95 — RED receipts (the resource-hygiene bar)

Parent tree: `5374a00` (the v0.2.0 STATUS closer). Captured 2026-11-09. One section per lane; GREEN receipts land in the issue comments, not here.

## R1 — the soak's fd pin

- **Absence receipt**: `git show 5374a00:ium-core/tests/soak_bounds.rs | grep -c fd` → **0** — the soak pinned journal/WAL/RSS only (the #34 record). `load_100.rs` already pinned fd (< 400 for 100 jobs, #66 lane 5); the hours-shaped leg did not.
- A module-based RED (`mod common;`) was attempted first and **resolved against the existing fixture module** (`tests/common/mod.rs` serves the ffmpeg real-data drills) — test passed, no failure to record. Superseded honestly: the lane follows the file-local helper style the suite already uses for `rss_kb` (duplicated deliberately; a shared fixture module would drag fixture dead-code into every consumer).
- **The canary caught TWO measurement flaws pre-landing** — the loop working as designed: draft one pinned the process fd *baseline* and failed under cargo's parallel test execution (`descriptors did not return after close: 10 vs baseline 6` — the soak's own sqlite fds were legitimately open); draft two pinned the reference window and the FULL-SUITE gate caught the soak's startup churn the standalone run never saw (`fd_probe_detects_real_descriptor_growth ... FAILED`). Final shape: symmetric ±1-churn deltas around the narrow read windows — the detector must SEE ~5 opens AND their closure; everything else is sibling noise. Both lessons live in the test's comment.
- **Detector-alive canary** (`fd_probe_detects_real_descriptor_growth`): 5 real handles → detector sees ≥ +5; close → returns to reference ±1. The pin cannot vacuously pass.
- GREEN (local, Linux): `SOAK-BOUNDS {"events":16391,...,"fd_delta":0,"bounds":{...,"fd":8}}` — the soak opens and retains ZERO descriptors across 16 k events; bound pinned at 8 (generous against sqlite's WAL/SHM pair, tight against per-event retention).

## R2 — the shell idle + RSS pin

- **Absence receipt**: at `5374a00`, `.github/workflows/ci.yml` contains **0** references to resource/idle assertions (grep = 0) and `scripts/resource_pin.sh` does not exist — the app-e2e job drove the shell and measured nothing about its own footprint.
- **Detector proof** (`--self-test`, runnable everywhere, no WebKit): synthetic busy child = **+2000 ms** vs sleeping child = **+0 ms** over a 2 s window — the sampler separates burn from idle; the live pin cannot vacuously pass.
- Live mode boots the real shell under xvfb (DMABUF disabled, the #35 lesson) and pins a 60 s quiet window: tree CPU-time delta ≤ 5000 ms (a 100% polling loop burns ~60 000) and tree RSS ≤ 1.5 GiB (WebKitGTK baseline is a few hundred MiB). Provisional-generous by design; the CI receipt tightens them.

## R3 — the long-haul plateau pin

- **Absence receipt**: at `5374a00` the bin's ONLY "plateau" match is an S4 doc comment ("the frontier sits at 25%") — the longhaul leg printed `[longhaul][persist]` RSS telemetry (the #34 record) but **asserted nothing**, and stress.yml contained **0** references to longhaul: the overnight shape was never wired into the scheduled lane.
- **Landed**: the telemetry task now collects the (t, RSS) sample series plus the fd span (first/last across pause AND fresh-process replay); post-leg the bin computes the least-squares RSS slope (warmup first 25% dropped) and fails the leg past **24 MiB/hour** (a 1 KiB-per-tick leak = 720 KiB/hour: caught); fd drift > 4 fails the leg; short legs loud-skip the plateau, never silently.
- **Local proof (full arc, short leg)**: `EVIDENCE {"fd_span":{"first":21,"last":13,"delta":0,"bound":4},"plateau":{"skipped":true,"reason":"samples 8 < 40"},"replayed_and_resumed":true,"state":"Done"}` — byte-exact readback, journal replayable, s3-test GREEN against pinned SeaweedFS 4.51.
- **The ≥ 40-sample plateau computation is CI's proof**: the new `long_soak` stress job (default 55 min: 30 MiB @ 8 KiB/s + pause + fresh-process replay, timeout 150 min) rides the Saturday schedule and dispatch. Sandbox note: background processes die when the tool session ends here — two 10-minute local legs died one minute in (partial evidence file, no process); the long-leg proof belongs to CI's runner, where the job owns it.
- YAML restructure caught on read-back pre-commit: the first edit landed `long_soak` between the realworld job's steps, orphaning its evidence-upload step — fixed before any commit.

## R4 — the sanitizer lane

- **Absence receipt**: `git show 5374a00:.github/workflows/sanitize.yml` → **absent**; no workflow references sanitizers (grep = 0).
- **Landed**: `sanitize.yml` — weekly (Sundays 04:00 UTC; stress owns Saturdays) + dispatchable; ASan + LSan (nightly, `-Zsanitizer=address`, `detect_leaks=1`) over (1) the full ium-core suite (env-gated wire tests skip loud without servers), (2) the canonical S3 wire suite vs pinned SeaweedFS 4.51, (3) the TUS wire suite vs pinned tusd 2.10.1. NOT per-push (the CI-economics rule: full nightly recompile + ~2-3× runtime).
- Scope: the engine is the leak surface the loop owns — a system WebKit under ASan would drown the signal in foreign allocations; the shell's honesty is R2's job.
- Verification: YAML-lint green locally; the first dispatch run is the lane's real proof (foreign-allocation false-positive risk handled on the record if it fires).

## CI-caught fixes (round 1, runs 42541818712 / 42541837469)

- **R2 — the sampler read the wrong stat field**: the live pin failed with `tree RSS 1885448087439360KiB` (~1.1 petabytes): `/proc/<pid>/stat` field `[23]` is not the RSS page count (`[21]` is) — the CPU-only self-test sailed through because the RSS path had no proof. Fixed: `[21]` + the self-test now guards RSS sanity (a real process is 1 MiB–64 GiB). The same receipt delivered R2's first honest number: **cpu_delta 420 ms over the 60 s quiet window — the booted shell idles at ~0.7% of one core.**
- **R4 — proc-macros cannot carry sanitizers**: `error: building proc macro crate with sanitizers enabled is not supported` on `zerofrom-derive`. Fixed: `CARGO_BUILD_TARGET: x86_64-unknown-linux-gnu` at the job level — an explicit target keeps host proc-macros out of the sanitized compilation, and covers every cargo call inside the harness scripts.

## R3 dispatch round 1 (run 42541835145) — the long soak's first REAL find

The 55-minute honest rehearsal of the owner's KB/s uplink (8 KiB/s bearer, 4-way fleet) FAILED: `frontier=0` at t+3296 s, `chunk_evicted` cascade, attempts exhausted, `resumed leg ended Failed`. The journal + proxy receipts (artifact `long-soak-evidence`) pin the mechanism:

- **Eviction medians poisoned by the socket-buffer burst**: the `chunk_evicted` events carry `median_bps` of **461 169 / 266 615 / 72 091 B/s on an 8 192 B/s bearer** — physically impossible without each lease's FIRST window measuring the client's kernel-buffer drain (hundreds of KB at localhost speed) instead of the wire.
- **The honest trickle then sits below 0.4×median for 2 windows** → evicted → re-lease re-bursts and re-dies → attempt budget exhausted. The proxy's `client abandoned PUT mid-body` lines at 82–129 s with 0.5–1.5 MB moved match the 2-strike arithmetic exactly.
- **Harness layer**: every abandoned PUT's `VICTIM-WIRE` shows the FULL 8 243–8 277 B/s while its peers starve at honest `bps: 0.0` — the shared bucket granted 250 ms of tokens per take, and the lead relay's wake phase monopolized it (`take()` drains the whole grant and re-queues first).

Two fixes, one per layer:

1. **Engine (the product bug)**: `ThroughputWatch` now DISCARDS each lease's primer window — it measures the kernel socket-buffer drain, not the wire, on ANY link where part-time ≫ window (the owner's real uplink qualifies; no proxy required). The primer still seeds the accounting so window 2 measures the true steady-state delta. Unit test extended: primer discarded, window-2 delta exact, primer bursts cannot leak.
2. **Harness (the test double)**: the shared bucket grants ONE 20 ms poll-slice per take (`rate × 0.02`, was ×0.25) — no single connection can outrun a cycle; the wake rotation shares the bearer by construction.

Full core suite green after the engine change (46/46 binaries). End-to-end validation = the next `long_soak` dispatch (the 55-minute leg must reach `Done` with an honest plateau receipt).
