# Design note — the aggregate-vs-single measurement drill (the one open acceptance item of #50)

Grounded in a **live wire rehearsal** on the pinned reference server (tusd
2.10.1 through `scripts/throttle_proxy.py`, `scripts/issue55_wire_rehearsal.sh`),
per the M3 discipline: wire facts BEFORE Rust.

## 1. Wire facts (captured live, this issue's vectors)

| # | Probe | Result on tusd 2.10.1 + the throttle proxy |
|---|-------|---------------------------------------------|
| 1 | `POST /files/` through the proxy (passthrough) | **tusd honors the request Host**: `Location: http://127.0.0.1:<PROXY>/files/…` — the shaped wire carries the WHOLE session (creations, PATCHes, final, verify). No behind-proxy machinery needed; the drill's endpoint is the proxy and the measurement cannot be bypassed. |
| 2a | per-conn cap C=64 KiB/s, single PATCH 256 KiB | 4.01 s = **64 KiB/s exactly** (the schedule is target-based; the 250 ms step is 6% of the window — noise, not shape) |
| 2b | per-conn cap, TWO concurrent PATCHes (own sessions) | per-flow 73 + 64 KiB/s, **aggregate 137 KiB/s ≈ K·C** — multiplies. This IS `LinkClass::PerFlowCap`, measured live. |
| 3a | shared bearer T=64 KiB/s, single PATCH 256 KiB | 3.95 s = 65 KiB/s |
| 3b | shared bearer, TWO concurrent PATCHes | per-flow 61 + 31 KiB/s, **aggregate 92 KiB/s < 1.5×T** — stays at the bearer. This IS `LinkClass::BearerCap`, measured live. |
| 4 | `OPTIONS` / creation `POST` (no body) in both modes | 1–3 ms — **no-body requests ride free**; the prober's creation phase never eats the measurement budget. |

Two rehearsal-caught instrument fixes (bugs the rehearsal itself exposed —
the drills exist to catch these):
- the shared bucket accumulated idle credit up to a 2 s burst ceiling → the
  first "bearer" PATCH measured 2× its own rate; the ceiling is now 250 ms —
  a measurement instrument shows the steady-state shape, not a boot burst;
- an upstream early-response (tusd 409s a stale offset without reading the
  body) killed the relay silently; the proxy now logs `UPSTREAM-GONE` and
  tears both ends down honestly.

## 2. Architecture (the drill, `ium-core/tests/concat_measurement.rs`)

Env-gated `IUM_CONCAT_MEASURE=1` (dispatch/manual — the PR lane skips loudly,
like the #45 ladder). The live tusd endpoint arrives as `IUM_TUS_ENDPOINT`
(the `tus-test.sh` pattern); the test spawns the Python proxy per leg
(`CARGO_MANIFEST_DIR/../scripts/throttle_proxy.py`) and points the job at
**the proxy** — every byte of the managed job rides the shaped wire (fact 1).

Three legs, one managed-job shape each (`create_job` → `run_job` →
`Done`, `IUM_LINK_PROBE=1`, real drive loop, AIMD waterline, concat
finalization, verify on the final URL, byte-exact read-back):

| Leg | Proxy shape | Seed | Expected (asserted) |
|-----|-------------|------|---------------------|
| control | per-conn C | 1 | the lane declares ONE flow → aggregate ≈ S1 (< 1.5×). **The instrument's honesty baseline**: this is the shape where the multiplication assertion MUST fail — the drill measures lane-ness, not wire-ness. |
| per-flow | per-conn C | 4 | `link_profile.class == per_flow_cap`, `recommended_fleet == 4`, `max_parallel == 4` journaled, N=4 partials, **aggregate ≥ 2.0×S1** (ideal 4×, recorded; the 2.0× bar is the prober's own classification line — pool granularity and runner load live between the two and the numbers are recorded, not hidden) |
| bearer | shared T | 4 | `link_profile.class == bearer_cap`, `recommended_fleet == 1`, the waterline parks workers above 1, **aggregate < 1.5×S1** (no multiplication — the declaration channel obeys the measurement) |

Every leg: Done + byte-exact read-back of the final URL (the concat lane
under measurement, not a mock) + the drill numbers printed as the record.

## 3. Sizing (why C = T = 64 KiB/s)

The managed prober call is `probe_link(…, k=4, None)` — the canary is the
64 KiB default. At C=64 KiB/s the single-stream window is 1.0 s (±1 tick =
±10%), the K-stream windows are 250 ms each (the tick floor lands ON the
target, no distortion) → per-flow ratio ≈ 4× (needs ≥ 2 ✓✓); bearer ratio
≈ 1.0–1.4× (needs < 1.5 ✓; the 16 KiB start credit cancels in the ratio —
rehearsal facts 3a/3b measured 1.42 at the larger 256 KiB scale, closer to
1.0 at 64 KiB). Leg sources: per-flow 4 MiB (measured piece ≈ 20 s × S1 =
1.25 MiB → 4 pool chunks → 4 concurrent flows), bearer/control 2 MiB
(waterline 1 anyway). Expected wall: ~90 s total for the three legs.

## 4. RED face

At HEAD the measurement does not exist: grep-proven — no drill output, no
assertion comparing a job's aggregate to the prober's S1 anywhere (`#45`'s
ladder asserts achieved≈wire for ONE flow; `#48`'s `prober_wire` asserts
classification on a mock, never a real job's aggregate; `#50`'s e2e asserts
byte-exactness, never rate). The instrument-honesty half of the RED: the
multiplication assertion evaluated against the control leg (seed=1,
per-flow wire) fails with aggregate ≈ S1 — pinned as RED evidence before
the GREEN run.

## 5. Close-out

Landing this drill closes #55 ("closes when the measurement is on the
record") and with it #50 — whose GREEN comment reserved exactly this item.
The drill output (all numbers, all three legs) goes on the record here and
on #50; STATUS.md row; one commit.
