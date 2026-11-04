# #47 design note — the stale-epoch poison + the silent-exit arm + reclaim-then-decide

## Root cause (two compounding defects, both verified against the RED run)

### 1. The stale-epoch poison — the driver's `resplit_rx` never consumes the epoch change

`drive_running` creates the resplit watch pair **once** (`manager.rs:710`) and the
driver itself never calls `changed()`/`borrow_and_update()` on it. Workers are
handed fresh clones each round — and a `tokio::sync::watch::Receiver` clone
**preserves the parent's unseen marker**. So after ANY resplit bumps the epoch,
every worker in EVERY later round sees `resplit_rx.changed()` complete
instantly on its first `select!` poll: `Step::ResplitUnderneath` aborts the
lease it **just journaled**, journaling nothing.

The drill, decoded:

- **Round 1**: the winner (single-worker lane) resplits, re-leases new piece 0
  (journals `Leased`), enters `select!` — and its OWN bump (unseen by its own
  receiver) fires instantly → `ResplitUnderneath` → journals nothing → **piece
  0 stranded, death #1**. It re-leases piece 1..15 and confirms them.
- **Round 2**: round-start reclaim fires (`leases_reclaimed`), the worker
  re-leases piece 0 (journals `Leased`), enters `select!` — the round-2 clone
  is taken from the driver's still-never-consumed receiver (seen marker still
  at epoch 0, current epoch 1) → fires instantly AGAIN → stranded, **death #2**,
  zero journal writes.
- Round 2 ends: `after == before` (spin guard) → Finishing →
  `verify_inconsistent` → Failed. Matches the RED chronology exactly
  (`leases_reclaimed` → `running→finishing` → `verify_inconsistent`).

### 2. The `Step::ResplitUnderneath` arm journals nothing by design

Its comment says "resplit_pending retired every lease … there is nothing to
journal or evict by idx" — true ONLY for leases taken BEFORE the winner's
rewrite. A lease taken AFTER the rewrite (the winner itself always re-leases;
the poison makes every later lease a fresh victim) is **live in the current
plan**, and its silent strand is a `Leased` row with no owner and no
resolution.

## Fix — three layers, each independently sound

a. **Consume the epoch at lease time** (closes the poison at its root): at the
   worker loop top, before leasing, `resplit_rx.borrow_and_update()` — mark the
   current epoch seen. A lease taken now is live in the current plan; only a
   NEW bump (a real resplit during THIS upload) may abort it. Both orderings
   safe: worker-leases-then-winner-rewrites → the rewrite retires the lease
   (arm (b) handles it); winner-rewrites-then-worker-leases → the worker leases
   from the NEW plan, correctly.

b. **The ResplitUnderneath arm checks lease identity** (the same guard the
   confirm/error/eviction arms already run): if the lease's `(begin,end)` still
   matches the current plan row, the lease was NOT retired — journal `Pending`
   (keeping lease-time confirmed bytes) and evict in-memory, all under the pool
   lock across the journal mirror (the lease-block invariant). If it
   differs/vanished, the winner's rewrite already reset the row — journal
   nothing (unchanged behavior). No silent strand for a live lease, ever.

c. **Driver reclaim-then-decide** (the issue's fix direction #2/#3 — makes the
   `pending == 0` exit unreachable while a reclaimable lease exists): after
   `join_all`, every worker has EXITED — any `Leased` row in the journal is
   definitionally ownerless within this driver. Reload the pool; if Leased rows
   exist → journal-reclaim them (Pending, keep confirmed bytes, the same rule
   the round-start reclaim applies) + `leases_reclaimed {"when":"round_end",
   "idxs":[…]}` → `continue` into a fresh round. Bounded against livelock:
   after 3 consecutive zero-progress reclaim rounds, journal
   `drive_pool_snapshot` and break — loud and honest, never spinning. And per
   acceptance #3, BOTH remaining break-to-Finishing paths (`pending == 0` at
   round start, the spin guard) journal `drive_pool_snapshot` when they leave
   non-uploaded bytes.

## What the fix does NOT do

- No journal-truth weakening: reclaims are journaled events; rows keep their
  confirmed bytes; transitions stay journaled before they execute (ADR-002).
- No retry-budget charge on a reclaim: a strand is not a failure.
- No protocol/adapter change, no drive-by refactor.

## Acceptance mapping

- Drill un-ignored reaches `Done` byte-exact → (a) lets piece 0's round-1 lease
  survive its own winner's bump; (b)+(c) catch any residual strand shape.
- No `verify_inconsistent` reachable from a resplit under an in-flight tus
  lease → (a) removes the false abort, (b) removes the silent strand, (c)
  self-heals any future silent-exit path.
- The `pending == 0` exit journals its pool snapshot → (c).
