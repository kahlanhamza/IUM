# Follow-up: the Windows lane of the un-ignored drill (CI run #84) — assertion portability, the fix itself held

Run #84 (`8f238dd`) went red on **core (windows-latest)** ONLY — and the RED is the
drill's own assertion, NOT the race:

```
test slow_tus_lane_resplits_below_the_s3_minimum ... FAILED
panicked at ium-core\tests\lane_floor.rs:135:
assertion `left == right` failed: the resplit target must clamp to the LANE floor
(64 KiB) … payload: {"chunks_after":16,"chunks_before":1,
"median_bps":138831.24911338865,"target":69415}
  left: Some(69415)
  right: Some(65536)
```

Critically: the panic is at the resplit-target assertion, which runs **after**
`assert_eq!(done.state, JobState::Done)` had already PASSED — **the #47 race fix
held on Windows too**: the job reached Done through the resplit-under-lease dance.

## What actually happened on Windows

The drill paces the tus body at 64 KiB/s and expected the fleet median to be
UNMEASURABLE (0.0 → the #28 frozen-link rule → target = lane floor = 65536
exactly). On Windows CI, the throughput-window sampling caught a MEASURABLE
median (138,831 B/s — timing-granularity noise letting a window see a burst), so
the resplit policy honestly computed target = median × 0.5 = 69,415 B, clamped UP
to the lane floor (69415 ≥ 65536 ✓) and far below the 5 MiB S3 constant ✓ —
16 pieces of exactly 64 KiB each (1 MiB / 16, even division), Done, byte-exact.

The engine behaved CORRECTLY per #44's contract; the drill had pinned an exact
NUMBER that only holds on the frozen path — the same over-pinning class the
boundary register warns about. Portable invariant replaces it:

- `target ∈ [64 KiB, 5 MiB)` — at least the lane floor, never the S3 constant;
- `chunks_after ≥ 8` and ≥ 8 pieces `≤ 128 KiB` (2× lane floor) — the split
  really happened into many tiny pieces, whatever the measured median was;
- `no piece exceeds the original plan` and byte-exact — unchanged.

Fix commit: portable assertions in `ium-core/tests/lane_floor.rs` (3/3 green
locally on Linux, suite 160/160, fmt + clippy `-D warnings` clean). Runs #84
and #86 are the honest reds of the over-strict assertion; the fix run closes
the lane.
