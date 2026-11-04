## GREEN evidence — the ladder, measured (local, entry-HEAD commit)

`IUM_THROTTLE_LADDER=1 cargo test -p ium-core --test throttled_ladder -- --nocapture`:

```
[ladder] 3 KB/s — the bad-moment wall's edge (#42, #44 lane floor):
         65536 bytes in 21.34 s (3070.5 B/s achieved), wire overhead 0.72%
[ladder] 8 KB/s — just above the old 8.7 KB/s wall line:
         131072 bytes in 16.01 s (8186.0 B/s achieved), wire overhead 0.36%
[ladder] 50 KB/s — a bad mobile uplink:
         524288 bytes in 10.26 s (51080.8 B/s achieved), wire overhead 0.09%
[ladder] 160 KB/s — the owner's 1.29 Mbps speedtest moment:
         1048576 bytes in 6.44 s (162883.6 B/s achieved), wire overhead 0.05%
test result: ok. 1 passed; 0 failed
```

What each number MEANS:

* **Achieved ≈ 99%+ of the wire at every rung** (3070.5 of 3072, 8186 of
  8192, 51081 of 51200, 162884 of 163840): the engine adds essentially
  ZERO duration of its own — the entire #42 wall removal, working.
* **Byte-exact readback at every rung** through the paced lane.
* **The #44 overhead honesty, MEASURED**: 0.72% at 64 KiB pieces
  (3 KB/s leg), falling to 0.05% at 1 MiB — the lane floor is not a
  scam; the per-PATCH head cost is recorded, not asserted.
* The 3 KB/s leg is the old wall's edge: a 64 KiB piece confirms in
  ~21 s — under the old 600 s wall this drill was impossible at CI
  timescales with any legal 5 MiB piece (it needed 29 min).

Wiring: stress.yml gains the `Throttled-link ladder 3/8/50/160 KB/s`
step (`IUM_THROTTLE_LADDER=1`) — weekly + dispatch, never the PR lane;
the PR lane keeps the compressed tus_slow_lane drills.
