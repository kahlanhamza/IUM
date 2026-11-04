## RED evidence — lane floor drill file

`ium-core/tests/lane_floor.rs` at HEAD (pre-#44), full `cargo test -p ium-core --test lane_floor`:

```
error[E0609]: no field `min_chunk_bytes` on type `ProbeReport`
```

The drill file pins two things the current engine cannot express:

1. `tus_probe_declares_the_lane_floor` — tus's ProbeReport must carry
   `min_chunk_bytes: Some(64 KiB)` through the ADR-011 declaration channel
   (64 KiB confirms in ~30 s at the owner's 3 KB/s bad-moment rate).
2. `slow_tus_lane_resplits_below_the_s3_minimum` — end-to-end: 1 MiB over
   two 512 KiB pieces, the whole tus lane paced at 64 KiB/s (global read
   pace on the loopback mock — the uniformly-slow-uplink shape, #18).
   The fleet median is 64 KiB/s, so the resplit targets ~32 KiB of wire.
   Expected (GREEN): the target clamps UP to the LANE floor (64 KiB), the
   pending 512 KiB piece splits into 64 KiB pieces, byte-exact finish.
   At HEAD: `execute_resplit` calls `plan(..., CHUNK_MIN)` and
   `resplit_pending(target, CHUNK_MIN)` — the target clamps to 5 MiB, the
   split cannot happen, and the assertions on the `chunk_resplit` payload
   (`target == 65536`) and on the 64 KiB pieces fail. The 5 MiB constant
   is being applied where NO protocol demands it (tus has no minimum);
   only S3's 5 MiB part minimum justifies it.
