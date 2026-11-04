# #47 GREEN evidence — fix commit `9cba6a5` (pushed as `8f238dd`)

Command: `cargo test -p ium-core --test lane_floor`
Result: **ok. 2 passed; 0 failed** (16.1 s) — `slow_tus_lane_resplits_below_the_s3_minimum`
reaches **Done**, byte-exact store, resplit target clamped to the LANE floor.

Stability: **3/3 consecutive green runs** (16.12 s / 16.13 s / 15.61 s) — the drill
exercises the resplit-under-lease path on every run.

Full gates on the committed tree (`8f238dd`, `scripts/gate.sh`): fmt clean · clippy
`-D warnings` clean · **160/160 core tests** (159 → 160: the drill is now a permanent,
un-ignored member of the suite) · ium-app vitest 9/9 · doc lane clean.

## What each fix layer contributed

1. **Epoch consumed at lease time** (`resplit_rx.borrow_and_update()` at the worker
   loop top) — kills the stale-epoch poison at its root: no lease taken after a
   resplit is aborted by the OLD bump anymore. This alone un-strands the drill.
2. **Identity guard in `Step::ResplitUnderneath`** — if a lease's `(begin,end)` still
   matches the current plan row, the lease was NOT retired by the rewrite: it journals
   `Pending` (keeping lease-time confirmed bytes) and evicts in-memory, all under the
   pool lock across the journal mirror. A retired identity stays unjournaled (the
   rewrite already reset its row). No silent strand for a live lease, ever.
3. **Driver reclaim-then-decide** — after `join_all` every worker has exited, so any
   `Leased` row in the journal is definitionally ownerless within this driver: the
   round-end path reclaims them (`leases_reclaimed {"when":"round_end","idxs":[…]}`)
   BEFORE any exit decision, bounded by `MAX_IDLE_RECLAIM_ROUNDS` (3) against
   livelock, and every non-clean break-to-Finishing exit journals a full
   `drive_pool_snapshot` (acceptance #3: the exit always leaves evidence).

## Acceptance checklist

- [x] The drill runs UN-ignored and reaches `Done` with byte-exact store
- [x] No `verify_inconsistent` path reachable from a resplit under an in-flight tus
      lease — (1) removes the false abort, (2) removes the silent strand, (3) self-heals
      any future silent-exit path before an exit decision can fire
- [x] The `pending == 0` exit journals its pool snapshot (`drive_pool_snapshot` — also
      wired into the spin-guard exit; per-chunk shape matches `verify_inconsistent`)

CI run #84 (`8f238dd`) is the closing run; the fix commit closes this issue.
