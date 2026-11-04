# #47 RED evidence — drill un-ignored at HEAD `cbc578d`, reproduced

Command: `cargo test -p ium-core --test lane_floor`
Result: **FAILED** (`1 passed; 1 failed`) — `slow_tus_lane_resplits_below_the_s3_minimum`
panicked at `assertion left == right failed: left: Failed, right: Done` (15.6 s).

Journal tail (the drill dumps the last 80 events newest-first; chronological order):

```
job_created (chunks:1) → probing → running
→ probe {max_parallel:1, min_chunk_bytes:65536, resumable:true}
→ chunk_resplit {chunks_before:1, chunks_after:16, target:65536, median_bps:0.0}
→ chunk_confirmed idx1..3 (full, confirm-full absorption)
→ chunk_busy_wait {consecutive:1, idx:N, wait_ms:1000} + chunk_confirmed for idx4..15
→ leases_reclaimed {}
→ state running→finishing
→ verify_inconsistent {"chunks":[[{"confirmed":0,"idx":0,"len":65536,"state":"leased"},
   {"confirmed":65536,"idx":1..15,"state":"uploaded"}...]],
   "detail":"reached Finishing with chunks not fully confirmed"}
→ state finishing→failed
```

Counts across the whole run: 15 `chunk_confirmed` events (idx 1–15), exactly
**one** event naming idx 0 (its initial lease) — no busy_wait, no confirm, no
error, no eviction ever resolves piece 0 after the resplit. Byte-exact store
for the pieces that DID land.

Decoded (full analysis in the design note): piece 0 is stranded TWICE —
death #1 in round 1 (the resplit winner re-leases the new piece 0 and its own
unseen epoch bump kills the fresh lease on the first `select!` poll, silent),
death #2 in round 2 (the round-start reclaim returns piece 0 to Pending, the
worker re-leases it, and the round-2 receiver clone — inherited from the
driver's never-consumed receiver — still carries the unseen marker, so the
first poll kills it again, silent). Round 2 ends zero-progress → spin guard →
Finishing → the verify gate fails the job loudly and honestly.
