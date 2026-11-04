# 07 — The Speed Architecture: getting to the bound honestly

*Issue #46's deliverable (M7 research track, owner mandate 2026-11-04: "the
purpose was speed and time … there is always a solution and paper
researches"). Input: the owner's research brief
([`07_Fast_Upload_Engine_Research_brief.md`](07_Fast_Upload_Engine_Research_brief.md))
— independently converged with the M7 filing; this document is the
engine-side synthesis: what is TRUE, what IUM already does, what M8 builds.*

## 0. The one law

**No algorithm exceeds the sum of the summoned bearers.** The ladder (#45)
measured IUM riding a shaped wire at 99%+ of the rate on every rung — the
engine's own overhead is ~zero. Everything below is about (a) filling the
share one flow leaves on the table, (b) summoning MORE bearers than a
single flow/interface can see, and (c) sending fewer bytes. Cause-D links
(the access line really is 3 KB/s) are served honestly: fewer bytes, more
links, or a honest "a USB stick wins" — the brief's §8 levers.

## 1. What the papers pin (verified sources)

| Fact | Source | What it means for IUM |
| --- | --- | --- |
| MPTCP v1 is shipped, standardized tech (Proposed Standard, 2020); subflows aggregate interfaces; Apple has run it since 2013 | RFC 8684; mptcp.dev | Multi-link aggregation is not exotic. IUM gets it WITHOUT kernel MPTCP: independent parts/sessions per interface (below). |
| The scheduler is the product: path managers + packet schedulers decide which subflow carries what | RFC 8684 §5; mptcp.dev concepts | IUM's chunk pool IS a scheduler — part→path assignment is a policy layer, not a protocol rewrite. |
| QUIC multipath is in the RFC-Editor queue (draft-ietf-quic-multipath-21, 2026-03) | datatracker | The HTTP/3 future has first-class multipath; benchmark-option per the brief's honest H3 caveat (bulk gains can be minimal). |
| TCP on asymmetric paths collapses for reasons that are NOT bandwidth: ACK theft, queue pressure upstream | RFC 3449 (BCP 69, Balakrishnan et al.) | The owner's 21:1 asymmetry is textbook. IUM's countermeasures: fewer/bigger acks per byte (multipart), byte-level resume so a collapse costs nothing already-sent, and the idle-progress tripwire (#42) instead of duration walls. |
| Erasure coding (RaptorQ) makes loss repair LOCAL — no round-trips | RFC 6330 | On lossy long-RTT tunnels, retransmits cost a full RTT each. FEC parity turns repair into arithmetic. An application-layer FEC lane is the Aspera-style existence proof (brief §Track 2). |
| tus concatenation: "concatenate multiple uploads into a single one enabling Clients to perform parallel uploads" — server-side optional | tus.io protocols; tus-node lacks it (community, 2025) | tus-lane parallelism has a standard path but per-server support. IUM declares what the server supports and never guesses (ADR-011). |

## 2. What IUM already does (M7, measured — not aspirational)

* **The wall is gone.** The 600 s total request timeout (structural
  non-convergence below ~8.7 KB/s) is replaced by idle-progress semantics:
  a request dies only when bytes stop arriving (#42).
* **Own-drift is absorbed, never fatal.** The realign dance is bounded and
  server-truth-driven; resplit-induced beyond-lease drift confirms in full
  on a single-writer lane (#43, #44).
* **The lane floor is lane-aware.** tus pieces go to 64 KiB (a piece
  confirms in ~30 s at the bad-moment rate; measured wire overhead 0.72%)
  while S3 keeps its protocol 5 MiB (#44).
* **The KB/s tail is pinned.** 3/8/50/160 KB/s through the real engine:
  99%+ of the wire at every rung, byte-exact (#45, stress.yml).
* **The link is measured, not guessed.** The prober (#41) answers the
  1-vs-K question by measurement: per-flow caps → fleet K; shared bearer →
  fleet 1; piece = 20 s of measured wire. The profile is journaled — the
  planner consumes data, never folklore.

## 3. The M8 ladder (each rung is independently shippable)

1. **Prober-driven planning** (ADR-012): the journaled `link_profile`
   feeds the planner — piece target and fleet come from measurement at
   job creation; re-probed on the job's own schedule (never mid-flight
   re-probes of a live session — ADR-002).
2. **AIMD fleet control** (the brief's §4.4): after the prober seeds the
   fleet, additive increase (+2) / multiplicative decrease (×0.7) on
   error-rate or RTT inflation, hard stop at plateau. Bufferbloat matters
   on a 21:1 asymmetric tunnel.
3. **Multi-path bonding** (the brief's §7 + RFC 8684's scheduler idea):
   bind workers to interfaces (`SO_BINDTODEVICE`), spread chunks across
   links — throughput approaches the SUM. IUM's parts are already
   independent; the prober already measures per-path rates.
   *Spike:* one host, two egress paths, part-parallel aggregate vs
   single-path — the M8 acceptance measurement.
4. **The tus concatenation lane**: when the server advertises
   `concatenation`, tus parallelism is standard; the adapter declares it
   and the planner obeys (ADR-011). Servers without it stay honest at
   `max_parallel = 1`.
5. **The FEC lane** (RFC 6330): application-layer RaptorQ parity over
   lossy tunnels — repair without round-trips. Ship as its own lane with
   the ladder's loss models (the #32 shapes) as the gate.
6. **Cause-D byte reduction** (brief §8): entropy-gated zstd; FastCDC
   content-defined chunking with a chunk-hash index so a re-upload sends
   only deltas; and the honest UI line when the line itself is 3 KB/s.

## 4. Non-goals (unchanged)

Exceeding the bearer sum. Parallelizing endpoints that cannot resume.
Trading honesty for a benchmark: every claim in this repo is a drill
output, and every failure shape is a filed issue with evidence.
