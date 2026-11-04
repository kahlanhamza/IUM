# RED evidence — issues #48 + #49 (M8 rungs 1–2: prober wiring + AIMD fleet)

Date: 2026-11-04 · Toolchain: cargo 1.99.1 · Tests written FIRST, no src changes.

## The RED

```
$ cargo test -p ium-core --test aimd_fleet --test prober_wire
error[E0432]: unresolved import `ium_core::fleet`
  --> ium-core/tests/aimd_fleet.rs:33:15
   |
33 | use ium_core::fleet::{fleet_for, AdaptiveFleet, FleetReason};
   |               ^^^^^ could not find `fleet` in `ium_core`
error: could not compile `ium-core` (test "aimd_fleet") due to 1 previous error
```

## What the RED proves (by absence)

1. **#48**: nothing in the engine measures a link at the probe boundary. `probe_link`
   (#41) is drill-proven but has ZERO production callers (grep: only `tests/prober_drills.rs`).
   No managed job ever journals `link_profile`; the planner never sees
   `recommended_piece_bytes`; `recommended_fleet` is dead data. RED tests:
   `prober_wire.rs::managed_tus_job_probes_journals_and_replans_to_the_measured_piece`
   (env opt-in, paced canaries, deterministic per-flow-cap shape) and
   `aimd_fleet.rs::no_link_probe_without_the_opt_in` (default-off face).
2. **#49**: the drive fleet is `pending.min(MAX_WORKERS).min(declared)` — a constant
   policy. There is no growth path at all: eviction/resplit/frozen-rule only shrink, so a
   job planned at the owner's 3 KB/s moment stays small forever after the link recovers
   54x. RED tests: the pure-decision arms (`AdaptiveFleet::step`: growth, decrease,
   overshoot, plateau, feedback gate, ceiling clamp; `fleet_for` composition) and the
   manager faces `scripted_errors_decrease_the_fleet_and_journal_the_reason` (8 scripted
   transient errors -> journaled `fleet_adjusted {from:8, to:5, reason:"errors"}` AND the
   next round provably runs 5 workers, per-epoch concurrency high-water), plus the two
   journal-seed faces (`link_profile` caps the first round at 2; last `fleet_adjusted`
   seeds a fresh manager at 3 — resume-safe by construction).

## The fix shape (design note = issue #48/#49 bodies)

- `ium-core/src/fleet.rs` (new, pure, offline): `fleet_for` + `AdaptiveFleet` with the
  brief §4.4 rule ported to round granularity — feedback gate (≥ 64 KiB AND ≥ 0.5 s),
  first feedback round = baseline, err_rate > 0.3 → ×0.7 (floor at 1), goodput > best×1.08
  → +2 (ceiling-clamped), goodput < best×0.9 → −1, plateau holds.
- Manager: seed `A = last fleet_adjusted | measured_fleet | ceiling` from the journal;
  per round `fleet = pending.min(ceiling).min(A)`; errors counted at the charge_budget
  arm; the decision journaled on every change; the link prober runs at the Probing
  boundary (tus, resumable, `IUM_LINK_PROBE=1` opt-in — every byte is sacred on a metered
  bearer), non-fatal by construction, and the measured piece re-plans while all-Pending
  (same journal-verified rule as #8), composed with the adapter cap (ADR-011 wins).
