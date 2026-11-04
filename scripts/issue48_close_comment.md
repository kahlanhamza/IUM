## CLOSED — the full loop, landed at `6d55fd4` (suite 160 → 170)

**RED first** (`scripts/issue48_red_evidence.md` in-tree): `probe_link` (#41) had ZERO
production callers — grep-proven. The engine never measured, never journaled
`link_profile`, never obeyed a recommendation. RED: `E0432 unresolved ium_core::fleet`
+ the absence-REDs (`prober_wire.rs`, `aimd_fleet.rs::no_link_probe_without_the_opt_in`).

**The fix, exactly per the design note in the issue body:**
- The `Probing` arm of `run_job` runs `probe_link` (tus, resumable,
  `IUM_LINK_PROBE=1` **opt-in** — one deliberate deviation from the body's default-on
  sketch: exact-wire drills (`tus_drill`'s `SIZE/2 ≤ WIRE ≤ SIZE/2+64 KiB`) and the app-e2e
  recorder assert exact byte totals, and on a metered bearer every byte is sacred. The
  prober is an instrument you switch on, not ambient traffic).
- `link_profile` journaled EXACTLY once at the probe boundary; a failed probe journals
  `link_profile_skipped {reason}` and the job proceeds — a lost measurement, never a lost job.
- The planner OBEYS: `apply_measured_piece` re-plans while all-Pending (journal-verified,
  the #8 boundary), **shrink AND grow** (plan phase — #27's grow rejection is mid-flight
  resplits, a different regime), composed with the adapter's per-call cap (ADR-011 wins).
- `measured_fleet` (journal re-derivation, never a live re-probe) seeds #49's waterline.

**GREEN:** `prober_wire.rs::managed_tus_job_probes_journals_and_replans_to_the_measured_piece`
— paced canaries (deterministic per-flow-cap: each connection its own 64 KiB/s), K=4
aggregate measures ~4× → `PerFlowCap`, fleet 4 journaled; the 4 MiB source re-plans from
1×4 MiB to 4 lane-legal pieces, all ≤ the measured recommendation; byte-exact through the
whole dance. Plus the default-off face pinned. Suite 170/170, fmt + clippy `-D warnings`,
gate GREEN on the committed tree.

**Honest follow-up:** the prober measures tus-shaped lanes; an S3-shaped prober (SigV4
canary parts) is a future delta on this issue's channel.
