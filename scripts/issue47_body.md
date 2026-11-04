## The race, from the #44 lane-floor drill

`ium-core/tests/lane_floor.rs::slow_tus_lane_resplits_below_the_s3_minimum`
(currently `#[ignore]`d as this issue's RED) drives: 1 MiB file, ONE planned
piece, the whole tus lane paced at 64 KiB/s, resplit policy compressed
(target_secs 0.5, slow_confirm 4 s). Sequence:

1. Worker leases the single 1 MiB piece (row → Leased), PATCH crawls at the pace.
2. ~4 s in: the frozen-link rule (#28, median unmeasurable) fires a resplit:
   1 → 16 pieces of 64 KiB (the LANE floor — #44's core, working), the
   in-flight lease is retired, `replace_chunks` rewrites all rows Pending.
3. The worker re-leases and the flow proceeds: pieces confirm (several via
   the confirm-full absorption of resplit drift), one 423 Busy-wait each
   around the abandoned request's server-side drain — **pieces 1–15 all
   reach Uploaded, byte-exact**.
4. **Piece 0's row ends `Leased` with `bytes_confirmed = 0` and NO
   journaled resolution** — no busy_wait, no confirm, no error, no eviction
   event ever names idx 0 after the resplit. The drive round then sees
   `pending == 0` (nothing Pending, not all Uploaded), breaks to Finishing,
   and the verify gate fails LOUDLY (`verify_inconsistent`) — honest, but
   the job is lost.

## Journal evidence (full chronology)

```
job_created (chunks:1) → probe (min_chunk_bytes:65536, max_parallel:1)
→ chunk_resplit {chunks_before:1, chunks_after:16, target:65536, median_bps:0.0}
→ chunk_confirmed idx1..3 (full, confirm-full absorption)
→ chunk_busy_wait/consecutive:1 + chunk_confirmed for idx4..15
→ leases_reclaimed {}
→ verify_inconsistent: idx0 {"confirmed":0,"len":65536,"state":"leased"}, idx1..15 uploaded
→ state finishing→failed
```

## What the race is (hypothesis to verify first)

The pre-resplit lease journaled `idx 0 → Leased`; the resplit's
`replace_chunks` (DELETE+INSERT) rewrote it Pending; SOMETHING re-journaled
`Leased` for the new piece 0 whose upload then ended through a path that
neither journals nor re-pools (candidates: a second epoch bump that never
executed a resplit; `watch::changed()` completing with Err on a dropped
sender surfacing as `Step::Aborted`/`ResplitUnderneath`; a lease journal
landing after the resplit's table rewrite despite the pool lock). The
drive loop's `pending == 0` exit then treats a Leased row as "held by a
live worker" — the same shape a crashed driver leaves, but WITHIN one
live driver.

## Fix direction

1. Reproduce minimally (the drill IS the reproduction; shrink if possible).
2. Make the drive round treat a `Leased` row with no live lease holder as
   reclaimable WITHIN the round (the same rule the round-start reclaim
   applies), or find and close the specific silent-exit window.
3. The `pending == 0 && !all_uploaded` break must not be reachable while a
   reclaimable lease exists — reclaim-then-decide.

## Acceptance

- [ ] The drill runs UN-ignored and reaches `Done` with byte-exact store
- [ ] No `verify_inconsistent` path reachable from a resplit under an
      in-flight tus lease
- [ ] The `pending == 0` exit journals its pool snapshot (evidence for any
      future inconsistent-pool shape)
