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
