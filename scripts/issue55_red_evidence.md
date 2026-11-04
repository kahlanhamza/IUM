## RED evidence — the aggregate-vs-single measurement drill (issue #55)

**Two faces, both live at HEAD (`8499f25`, before the drill existed).**

### 1. The measurement does not exist — grep-proven absence

No drill output, no comparison of a real job's aggregate against the
prober's single-stream measurement anywhere in the tree:

* `scripts/` has no aggregate-vs-single drill (the #45 ladder asserts
  achieved≈wire for ONE flow; nothing measures a lane's aggregate);
* `ium-core/tests/` has no assertion comparing a job's recorded goodput
  to a journaled `link_profile` (`prober_wire` classifies on a paced MOCK
  and asserts the replan, never a real job's aggregate; `tus_concat`
  asserts byte-exactness, never rate);
* `docs/07` §3 names the aggregate-vs-single measurement as the M8#3
  acceptance — an unmeasured claim, not a record.

### 2. The instrument is honest — the multiplication assertion FAILS on a single-flow lane

The RED flip: the per-flow leg's multiplication assertion
(`aggregate ≥ 2.0×S1`) evaluated against the CONTROL shape (seed=1 → the
classic lane declares exactly one flow on the per-flow-capped wire), live
vs tusd 2.10.1 through `--per-conn-rate 65536`:

```
[measure] control(seed=1, per-flow wire): S1=251.1 KiB/s SK=9836.8 KiB/s
          class=per_flow_cap rec_fleet=4 max_parallel=1 wall=33.3s
          aggregate=63004.8 B/s (61.5 KiB/s, 0.25x S1) [2097152 bytes]
panicked: RED-flip: control aggregate 63005 B/s vs S1 257147 —
          a seed-1 lane cannot multiply a per-flow cap
test result: FAILED
```

The aggregate rode ONE flow (61.5 KiB/s ≈ the cap) while the wire itself
was per-flow-capped — exactly the shape where a lane that rides bearers
would have multiplied. The drill detects lane-ness, not wire-ness.

**Rehearsal-caught instrument bugs (fixed before GREEN, each pinned in the
rehearsal script):**

1. The shared bucket accumulated idle credit up to a 2 s burst ceiling —
   the first "bearer" PATCH measured 2× its own rate (127 vs 64 KiB/s).
   Ceiling is now 250 ms: a measurement instrument shows the steady-state
   shape, not a boot burst.
2. The pacing schedule waits BEFORE READS, so a body that fits the socket
   buffer crossed faster than its cap — the 64 KiB canary measured
   261 KiB/s against a 64 KiB/s cap (one 64 KiB read, zero waits), and
   the 16 KiB K-canaries crossed at wire speed. The relay now pays its
   schedule debt before returning: the end-to-end rate IS the cap, for
   bodies of any size. The victim-drill semantics (unread-socket
   backpressure mid-body, the #28 wire meter) are untouched — the debt
   applies only at body completion.
3. An upstream early-response (tusd 409s a stale offset without reading
   the body) killed the relay silently; the proxy now logs
   `UPSTREAM-GONE` and tears both ends down honestly.

With the instrument honest, the flip was **re-taken** on the fixed wire
(S1=63.7 KiB/s — the cap itself) and failed identically:

```
[measure] control(seed=1, per-flow wire): S1=63.7 KiB/s SK=249.0 KiB/s
          class=per_flow_cap rec_fleet=4 max_parallel=1 wall=33.3s
          aggregate=63010.2 B/s (61.5 KiB/s, 0.97x S1) [2097152 bytes]
panicked: RED-flip: control aggregate 63010 B/s vs S1 65222 —
          a seed-1 lane cannot multiply a per-flow cap
test result: FAILED. 0 passed; 1 failed
```

The aggregate rode ONE flow (0.97×S1) on a wire the prober classified
`per_flow_cap` — the shape's truth, not a measurement artifact.
