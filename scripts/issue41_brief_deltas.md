## Owner research brief ingested → delta extraction for the prober (#41)

The owner supplied `docs/07_Fast_Upload_Engine_Research_brief.md` (commit cb9b8ff) — an independent survey of the "IDM for uploads" design space. It converges with this issue's design and adds concrete mechanics. What #41 takes from it:

**Confirmed by the brief**
- The CURVE method: probe with 1, 2, 4, 8, 16 parallel streams; throughput rising-then-flat ⇒ per-flow cap (causes A/B), flat-from-1 ⇒ bearer cap (cause D). This is exactly the classify step this issue pins.
- "Diagnose first, then pick the strategy" is the correct architecture (brief §1: A–C give 10×–1000×, D cannot be beaten by software — only by fewer bytes or more links).

**Corrections this issue makes to the brief**
- The brief's probe payload (8 MiB × up to 16 streams) is impossible on the owner's bad-moment link: 8 MiB at 3 KB/s = ~45 min per stream. The probe must be RATE-ADAPTIVE: start at the lane floor (64 KiB, #44), measure, then scale the probe so each leg stays inside a ~10–30 s budget. The curve is the same; the payload adapts.

**Adopted as follow-on (post-classification policy)**
- The brief's §4.4 AIMD concurrency controller: additive increase (+2), multiplicative decrease (×0.7) on error-rate or RTT-inflation > 2× baseline, hard stop at plateau (more streams past the line limit only add contention), N_max cap + "be gentle" mode. That is the fleet-sizing policy AFTER the prober classifies the link — phase 2 of this issue or the first M8 item.
- RTT-inflation (bufferbloat) guard matters specifically on the owner's tunnel (50 ms RTT, 21:1 asymmetry).
- Hedged requests (speculative re-issue of a slow chunk on another connection, keep first finisher): noted for M8 — but ONLY legal on per-flow-cap links; on a bearer-capped link a hedge double-sends bytes the bearer doesn't have.
