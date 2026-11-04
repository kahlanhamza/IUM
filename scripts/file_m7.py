#!/usr/bin/env python3
"""File milestone M7 (Speed architecture) + its issues, idempotently.

Owner mandate 2026-11-04: the product's purpose is SPEED and TIME, not only
survival. Two field measurements of the SAME tunnel within one hour:
  * NapsternetV uplink counter 930.1 KB over ~5 min of pushing  -> ~3 KB/s
  * Google speed test, same link, 17 min later                  -> 1.29 Mbps up (161 KB/s)
    (27.1 Mbps down / 50 ms / Singapore / VPN / 4G+)
A 54x swing on one link in one hour is the strongest possible argument that
the engine must MEASURE the link and choose lane + fleet from measurement —
the one trick IDM never needed (downloads don't choose protocols).

Repo token comes from .secrets/gh_token (never printed, never committed).
"""
import json
import sys
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path(".secrets/gh_token").read_text().strip()

MILESTONE_TITLE = "M7 — Speed architecture: measured profiles → the IDM brain"
MILESTONE_DESC = (
    "Owner mandate 2026-11-04: the purpose is speed and time. Same tunnel, one hour apart: "
    "3 KB/s uplink moment vs 1.29 Mbps uplink (54x swing), 21:1 down/up asymmetry. "
    "M6 proved reliability; M7 adds the missing organ — measure the link, classify the "
    "bottleneck, pick lane + fleet from measurement, and stop paying walls that kill healthy flows."
)

ISSUES = [
    {
        "title": "#41 Link prober: classify the bottleneck at probe time, pick lane + fleet from measurement",
        "body": """## Purpose
The one IDM trick we lack: IDM just opens many connections; an upload manager must CHOOSE lane and flow count, because the transport protocol constrains it (tus = 1 sequential offset per session, by protocol) and because the bottleneck's POSITION decides whether parallelism multiplies anything.

## Measured motivating evidence (owner field data, 2026-11-04)
| Moment | Uplink measured | 2 GB single-flow |
|---|---|---|
| NapsternetV counter (bad moment) | ~3 KB/s | ~9.4 days |
| Google speed test (good moment) | 1.29 Mbps = 161 KB/s | ~4.1 h (tunnel ovh incl.) |
| Asymmetry | 27.1 Mbps down / 1.29 up = 21:1 | — |

## The fork the prober must decide
* **Per-flow cap** (common on carrier/tunnel paths): K flows multiply → measured aggregate decides fleet. On S3 multipart this is IDM's segment pool verbatim.
* **Total-bearer cap**: parallel changes nothing (3 KB/s split N ways is still 3 KB/s) → fleet=1 minimizes atomic-part waste; tus/survival lane preferred.
* **Fat pipe** (ordinary links): current defaults are already fine; prober confirms in ~20 s.

## Design sketch
1. After `probe`, prober measures: (a) single-stream rate over ~10 s, (b) K-stream aggregate (K=4) over ~10 s. `parallel_gain = agg/single`.
2. Classification (pure fn, unit-tested): gain ≥ 1.5 → per-flow-capable → fleet = min(MAX_WORKERS, argmax K); gain < 1.2 → total-bearer → fleet 1 (S3) / 1 (tus, protocol anyway); else → default behavior.
3. Decision journaled in the probe event (ADR-011 channel extension: `measured_bps`, `parallel_gain`, `fleet`), re-derived from the journal every drive round — same discipline as `declared_max_parallel` (#13).
4. `create_job`'s max_conns becomes a CEILING; the measured fleet is the runtime truth. UI renders "fleet: N (measured)".
5. tus jobs: prober still runs (its measurement feeds #42's lease guard + honest ETA), fleet stays protocol-locked at 1.

## RED first
Manager today clamps fleet ONLY by adapter declaration; a measurement-keyed clamp does not exist. Test: loopback recorder with per-connection throttle shim (two throttled conns vs one), assert the journaled decision and the enforced fleet.

## Acceptance
- [ ] Prober module, decision pure fn unit-tested
- [ ] Decision journaled in probe event; journal re-derivation on every round
- [ ] Fleet clamp enforced in the drive loop from measurement
- [ ] Live drill vs loopback recorder (throttled 1-vs-2 conns) proves multiplication is detected
- [ ] Zero regressions: 152 core tests green""",
    },
    {
        "title": "#42 tus client: replace the 600 s total request timeout with an idle-progress timeout",
        "body": """## The wall, precisely
`TusAdapter::new` builds reqwest with `.timeout(600 s)` — a TOTAL per-request cap including body streaming. A 5 MiB piece (CHUNK_MIN) needs ≥ 600 s whenever the uplink is below **8,738 B/s**. Below that line NO legal piece size fits in one attempt:
* every first PATCH dies at 600 s having pushed ≤ 1.8 MB;
* the realign re-PATCH dies the same way;
* the second Conflict maps to `Unknown` → `Action::Fail` → `Failed` is TERMINAL.
**The tus lane is structurally non-convergent below ~8.7 KB/s.** Owner's bad-moment profile (3 KB/s) is exactly there.

## Design
1. Drop `.timeout(600)`. Keep connect timeout.
2. Wrap the PATCH body stream (FramedRead over the source) with an IDLE timeout: fail only if **no bytes flow for N s** (default 30 s — consistent with the eviction window). Healthy slow flows then stream a 5 MiB piece for 29 min untouched.
3. A total-lease guard SCALED by measured rate (from #41's probe event) stays available as a backstop; the supervision layer (frozen-link rule #28, eviction) already owns stalls — the request layer must stop owning duration.
4. Timeout error maps to `Transient` (counted retry), never `Unknown`.

## RED first
Loopback tus server throttled to ~2 KB/s (recorder `throttle` param): current HEAD fails the job in ~20 min with the Unknown classify; assert that failure mode FIRST (documented RED), then flip to idle-timeout and assert convergent completion.

## Acceptance
- [ ] RED pinned: 2 KB/s lane, current code, job Failed at the wall (posted on this issue)
- [ ] GREEN after: same lane completes; pieces confirm; journal converges
- [ ] Idle timeout fires on a genuinely silent socket (no infinite drip hang)
- [ ] Wall never binds at ≥ 3 KB/s healthy flows (ladder test at 3/8/50/160 KB/s)""",
    },
    {
        "title": "#43 tus realign: absorb own-drift instead of Unknown→Fail (the timed-out PATCH's own bytes)",
        "body": """## The misclassification
`upload_chunk`'s realign maps a second `Conflict` to `Unknown("409 again after realign — server moved under the lease")` → `Action::Fail`. The rationale assumes a single-worker lease means nobody else writes — true. But on slow links the server "movement" is **our own timed-out PATCH's bytes landing** (tusd persists what arrived before the socket died). That is drift to absorb, exactly like the first realign, not a client bug to fail loudly on.

## Design
1. After a failed realign PATCH: HEAD. If `server_offset ≥ realign_start` and `≤ lease.end` → own-drift: count the waste (issue #37 reckoning), re-PATCH from the server offset.
2. BOUND the dance: ≤ 4 realigns per lease; beyond that → `Transient` (budget counts it; the chunk re-leases from journaled truth). Never `Unknown` for own-drift — reserve it for offset BEYOND the lease (someone else genuinely wrote there).
3. Journal convergence invariant unchanged: only server-acked offsets are journaled.

## RED first
Wire test: PATCH allowed to push 1.8 MB then killed by the client at a 600 s-equivalent boundary (recorder kill-switch mid-body), realign re-PATCH from the landed offset — current code returns the Unknown classify; after the fix the lease converges with `waste` measured.

## Acceptance
- [ ] RED pinned and posted
- [ ] Convergent resume; bounded dance; no livelock
- [ ] `Unknown` still fires for a server offset beyond the lease end (the real protocol violation)
- [ ] Waste accounting honest in every branch""",
    },
    {
        "title": "#44 Lane-aware chunk floor: tus may legally go below the 5 MiB S3 minimum (ADR-011 channel)",
        "body": """## Why a global floor is wrong
`CHUNK_MIN = 5 MiB` exists because S3 multipart requires ≥ 5 MiB per non-final part. TUS has **no protocol minimum** — the floor is being applied where no protocol demands it. At 3 KB/s a 5 MiB piece confirms every ~29 min: feedback latency and drop-waste scale directly with piece size, and the resplit cannot help (it floors at CHUNK_MIN).

## Design (ADR-011: adapter declares, planner obeys)
1. `ProbeReport` gains `min_chunk_bytes: Option<u64>` (next to `max_chunk_bytes`, `max_parallel`). Tus declares e.g. `Some(64 KiB)`; S3/HTTP declare `None` → engine default CHUNK_MIN.
2. `plan_capped`/`resplit_pending` take the LANE floor, not the global constant. The #27 floor-regime algebra, #5 invariants (exact cover, no gaps/overlaps) and the invariant fuzz all run against the lane floor.
3. Overhead honesty: at 64 KiB pieces the tus per-PATCH header overhead is ~0.5% — measured and recorded in the drill, not asserted.
4. The prober (#41) closes the loop: measured rate → suggested piece target (30 s of wire) → clamped to the LANE floor.

## Acceptance
- [ ] Declaration plumbed end-to-end; S3 unchanged (5 MiB still enforced)
- [ ] plan/resplit/fuzz tests parameterized over both floors
- [ ] Live tus drill at 64 KiB pieces: zero re-upload on kill, honest overhead number recorded""",
    },
    {
        "title": "#45 CI throttled-link drill: pin the KB/s tail forever (the wall was invisible at CI speed)",
        "body": """## Why this drill must exist
The M6 probes covered loss, RTT, RST and B/s LADDERS — but every lane still ran above the ~8.7 KB/s line where #42's wall binds. The tail was discovered by the OWNER IN THE FIELD (3 KB/s tunnel moment), not by CI. That is the exact gap drills exist to close.

## Design
1. Rate model in the loopback recorder (`throttle` param already exists — extend to steady B/s pacing) or `throttle_proxy.py` gains a `rate` impairment.
2. Drill (stress.yml, minutes-scale): 512 KiB–2 MiB file through tus at **3 KB/s** and at **8 KiB/s** (the wall's edge); asserts: convergent Done, zero-re-upload after a mid-body kill, honest waste accounting, ETA sane.
3. Ladder legs at 3 / 8 / 50 / 160 KB/s double as the #42 acceptance lane and a permanent regression net for the resplit floor algebra (#44).
4. The S3 leg at 3 KB/s pins the fleet-1 rule (#41) — parallel parts must NOT be opened when the bearer is total-capped (each atomic part would outlive its timeout and confirm nothing).

## Acceptance
- [ ] RED against current HEAD (documented: tus leg fails at the wall) posted before the fixes
- [ ] GREEN after #42 + #43 (+ #44 for the 64 KiB leg)
- [ ] Runs in stress.yml; never blocks per-push CI on duration""",
    },
    {
        "title": "#46 Research track: multipath aggregation + FEC — getting to the bound honestly (M8 candidate)",
        "body": """## Owner directive
"There is always a solution and paper researches." Correct — and the research has names. Honest framing first: **no algorithm exceeds the sum of the summoned bearers.** What research delivers is (a) reaching the bound TCP leaves unfilled, and (b) SUMMONING MORE BEARERS than one flow/interface can see.

## Track 1 — Multipath aggregation (strongest, most product-shaped)
* MPTCP (RFC 8684) is shipped tech (Linux upstream; Apple uses MPTCP for Siri). Wi-Fi + cellular uplinks aggregate into one connection.
* S3 multipart maps 1:1 onto multipath: each part is an independent flow — IUM can aggregate paths WITHOUT kernel MPTCP by assigning parts to different interfaces/exits. The prober (#41) already measures per-path rates; part→path scheduling is a scheduler, not a protocol change.
* Deliverable: ADR + spike measuring part-parallel aggregate across two paths vs single path.

## Track 2 — FEC / UDP lane for lossy-tunnel regimes (the Aspera existence proof)
* IBM Aspera FASP: UDP + RaptorQ-style erasure coding + rate control holds near-line-rate on lossy/long-fat paths where TCP collapses. The owner's tunnel (lossy, 50 ms RTT, 21:1 asymmetric) is exactly FASP's home regime.
* Deliverable: design note docs/09 + ONE measurable spike: FEC-coded loopback transfer at 5% loss vs TCP baseline. No product promise until measured.

## Track 3 — Saturation hygiene (cheap, immediate)
* Resplit (30 s pieces) + eviction already keep the pipe full; endgame racing exists for the S3 tail. The prober's ETA from measured rate replaces the UI's lie-by-division on fluctuating links.

## Acceptance
- [ ] docs/09 design note with the honest bound framing
- [ ] Track-1 spike measured (two-path aggregate vs single)
- [ ] Track-2 spike measured (FEC vs TCP at 5% loss)
- [ ] Verdicts recorded — including "not worth it" if the numbers say so""",
    },
]


def api(method, path, body=None):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}{path}",
        method=method,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
        data=json.dumps(body).encode() if body is not None else None,
    )
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read().decode() or "{}")


def main():
    # 1. milestone (idempotent by title)
    ms = api("GET", "/milestones?state=all")
    m7 = next((m for m in ms if m["title"] == MILESTONE_TITLE), None)
    if m7 is None:
        m7 = api("POST", "/milestones", {"title": MILESTONE_TITLE, "description": MILESTONE_DESC, "state": "open"})
        print(f"milestone created: #{m7['number']} {m7['title']}")
    else:
        print(f"milestone exists: #{m7['number']}")
    # 2. close M6 (id 5) — all its issues are closed, run #75 green
    m6 = next((m for m in ms if m["number"] == 5), None)
    if m6 and m6["state"] == "open":
        api("PATCH", "/milestones/5", {"state": "closed"})
        print("milestone 5 (M6) closed — complete per run #75")
    # 3. issues, idempotent by title
    existing = []
    page = 1
    while True:
        batch = api("GET", f"/issues?state=all&per_page=100&page={page}")
        if not batch:
            break
        existing.extend(i["title"] for i in batch)
        page += 1
    for spec in ISSUES:
        if spec["title"] in existing:
            print(f"issue exists: {spec['title'][:60]}")
            continue
        created = api("POST", "/issues", {
            "title": spec["title"],
            "body": spec["body"],
            "milestone": m7["number"],
            "labels": ["enhancement"],
        })
        print(f"filed: #{created['number']} {created['title'][:70]}")


if __name__ == "__main__":
    sys.exit(main())
