# GREEN — the concatenation lane is live (implementation on `3b69b79`)

**Suite:** `tests/tus_concat.rs` **10/10 LIVE vs tusd 2.10.1** + the classic
wire suite **12/12 unchanged** + 32 offline suites green + fmt + clippy
`-D warnings` clean. Gate GREEN on the committed tree; pushed to `main`
(CI runs free on the public repo — verdict inline, no cron).

**What the code does now:**

| Piece | Shape |
|---|---|
| Discovery | `OPTIONS` at probe; `Tus-Extension: concatenation` → journaled `concatenation: true` in the probe event; **discovery failure degrades to the classic lane** (a job never dies on an optional probe) |
| Session topology | the journaled token carries it: plain URL = classic, JSON `{"concat":true,"partials":[…],"final":…}` = concat — pre-#50 journals re-derive byte-identically |
| Fleet seed | `probe_fleet(spec, seed)` (new trait method, default = `probe`); the host seeds from the **journaled `max_conns`** (the create-time plan is CHUNK_MIN-floored — the pool alone under-seeds small files); the adapter clamps N ≤ ceil(total/64 KiB) and declares `max_parallel: Some(N)` on the #13 channel — prober + AIMD adapt WITHIN it |
| Parallelism | partial *i* owns `[i·L, min((i+1)·L, total))`; chunks route their intersection; a straddling chunk is two segments; per-partial async Mutex — **the SESSION stays sequential, the LANE is parallel**; the fresh-partial straddle pushes the neighbor's bytes from the server's offset (absolute source mapping `base + local`), confirm counts only the segment's span |
| Finalize | `finalize(spec)` (new trait method, default no-op); manager journals `concat_final` intent BEFORE, `done` + final URL AFTER (tusd re-POSTs create a SECOND final — the journaled URL is never re-POSTed; orphan bound: one inert duplicate per kill in the done-window); the final URL re-journaled as the newest probe event so resume re-derives (ADR-002) |
| Verify | gates on the FINAL URL (`offset == length == source_size`, eager materialization); before finalize it honestly reports "not finalized" — verify refuses to lie |

**The e2e leg** (real drive loop): `create_job(spec, 3)` → probe_fleet (3
partials) → drive → Finishing → finalize → verify → **Done**, every byte
confirmed exactly once, journaled: `concatenation:true`, `max_parallel:3`,
`concat_final` intent+done.

**Honest deferred item:** the acceptance's aggregate-vs-single measurement on a
per-flow-capped link — filed as **#55** (proxy + prober classification + the
real job's recorded aggregate). #50 stays open until that measurement is on the
record; everything else in the checklist is done above.

Next rungs when #55 lands: #51 multi-path bonding · #52 RaptorQ FEC · #53
entropy-gated zstd.
