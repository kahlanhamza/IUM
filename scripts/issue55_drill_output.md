## GREEN — the aggregate-vs-single measurement, on the record (issue #55)

**Command:** `bash scripts/tus-test.sh --measure` (boots pinned tusd
2.10.1, exports `IUM_TUS_ENDPOINT`, runs `IUM_CONCAT_MEASURE=1 cargo test
-p ium-core --test concat_measurement -- --nocapture`).

**The record — three legs, every number measured, nothing asserted
blindly:**

```
[measure] control(seed=1, per-flow wire): S1=63.7 KiB/s SK=248.5 KiB/s
          class=per_flow_cap rec_fleet=4 max_parallel=1 wall=33.3s
          aggregate=63005.8 B/s (61.5 KiB/s, 0.97x S1) [2097152 bytes]
[measure] per-flow(seed=4, per-conn cap): S1=63.7 KiB/s SK=248.1 KiB/s
          class=per_flow_cap rec_fleet=4 max_parallel=4 wall=17.3s
          aggregate=241897.8 B/s (236.2 KiB/s, 3.71x S1) [4194304 bytes]
[measure] bearer(seed=4, shared T): S1=70.3 KiB/s SK=62.8 KiB/s
          class=bearer_cap rec_fleet=1 max_parallel=4 wall=35.5s
          aggregate=59093.7 B/s (57.7 KiB/s, 0.82x S1) [2097152 bytes]
test result: ok. 1 passed; 0 failed
```

**What each leg MEANS:**

* **control (seed=1, per-flow wire): aggregate = 0.97×S1.** The classic
  lane declares exactly one flow (`max_parallel=1` journaled) and rides
  one C — the instrument's honesty baseline. The LINK classifies
  `per_flow_cap` even so (the measurement never lies about the wire); the
  RED flip proved this is the shape where a multiplication assertion
  fails.
* **per-flow (seed=4, per-conn cap 64 KiB/s): aggregate = 3.71×S1.** The
  lane declared 4 partials (`max_parallel=4` on the ADR-011 channel), the
  prober classified `per_flow_cap` and recommended fleet 4, the AIMD
  waterline seeded at 4 — and the real drive loop moved 4 MiB at
  236 KiB/s where one flow moves 64. The ideal is 4.00×: the 0.29 gap is
  the drill's own honest arithmetic — the wall clock includes the ~1.3 s
  probe phase and the finalization/verify tail that a pure drive-rate
  meter would exclude (4 MiB / 16 s of pure drive = 4.00×; the recorded
  3.71× is the whole-job truth, per the drill-shape rule: every claim a
  recorded output, never an inline number).
* **bearer (seed=4, shared bearer 64 KiB/s): aggregate = 0.82×S1.** The
  prober classified `bearer_cap` and recommended fleet 1; the waterline
  parked the fleet at 1 (the lane still declared 4 partials — the
  declaration channel obeys the SERVER, the waterline obeys the
  MEASUREMENT); parallelism added nothing to one bearer. The 0.82 (below
  1.0) is the canary's 250 ms start-credit making S1 mildly optimistic —
  the honest note the drill records instead of hiding.

Every leg: Done, every byte server-confirmed exactly once, byte-exact
read-back of the verified URL (final URL on the concat legs, the session
URL on the classic control).

**Gates:** fmt clean, clippy `-D warnings` clean, suite **181/181** green
(180 + the drill), gate GREEN on the committed tree.

**Closes the loop:** #55's acceptance list is fully checked — the drill
script is `scripts/tus-test.sh --measure` + `tests/concat_measurement.rs`
(wired through the #45 instrument `scripts/throttle_proxy.py`, now with
the per-conn/bearer measurement shapes); the prober classifies the shaped
link live; aggregate > single is on the record above and pinned on #50;
the BearerCap fallback is recorded (the lane stays fleet-1 and the drill
proves it). #50's last open acceptance item is done — the rung closes.
