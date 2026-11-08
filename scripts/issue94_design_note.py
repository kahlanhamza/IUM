#!/usr/bin/env python3
"""Post the full design note on #94 (the speed-cap lane)."""
import json
import sys
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """## Design note (full — the lane is open)

The sketch held against the wire; three refinements from reading the drive
loop (manager.rs, the worker select! around `adapter.upload_chunk`):

### 1. The seam: a pre-upload, cancellation-safe budget acquire

The byte pump lives INSIDE each adapter (nine of them build their own request
bodies) — a reader-wrapper would be nine adapters of surgery. The honest v1
is **chunk-granular pacing at the one shared seam**: right before
`upload_chunk`, the worker acquires the lease's byte length from a global
token bucket. The acquire sits in its own `select!` (biased; abort + resplit
channels) so a capped wait stays cancellation-safe and resplit-aware — the
same reflexes the upload itself has.

**Why this is eviction-safe by construction**: the eviction ticker votes on
progress INSIDE the upload's select! window. The cap's wait happens BEFORE
that window — a waiting worker is simply between leases (the pause
contract's own vocabulary), so no `bps ≈ 0` sample can ever be a cap-wait
in disguise → **zero false evictions**. Once uploading, the wire runs full
speed, so `below_floor` sees real throughput, never the cap.

**Emergent granularity**: a capped fleet measures ~cap as its live rate; the
driver's own resplit logic (#27/#28) then reshapes chunks toward the lane
floor — so at a 50 KB/s cap the leases shrink to tens of KiB and the pacing
granularity tightens automatically. The burst-window objection (a 5 MiB
chunk at 50 KB/s = a 100 s wait then an instant burst) dissolves: the same
mechanism that made the slow-link ladder honest makes the capped ladder
fine-grained.

### 2. The honest persistence story (a correction to the sketch)

The sketch said "journaled". Wire fact: the manager's existing policy knobs
(`set_policy`, `set_eviction_policy`, `set_resplit_policy`) are RUNTIME
state — none are journaled; the journal is job truth, not engine config.
The cap follows the policy pattern: a manager-level knob
(`set_rate_cap(Option<u64>)`, bytes/sec; None = unlimited), persisted
SHELL-side (localStorage, the theme pref's pattern) and applied at boot —
the shell re-asserts it every start. No journal schema changes; zero
migration risk.

### 3. The bucket itself

Lazy lazy-computed refill, no background task: under the lock,
`available = min(cap, available + elapsed·cap)`; `acquire(n)` sleeps
`(n − available)/cap` inside the cancellation-safe select! loop and
re-checks. Deterministic under `#[tokio::test(start_paused = true)]`
virtual time. Shared across ALL jobs (the cap is the SUM across parallel
flows — the #46 vocabulary: the cap divides the bearer sum, it must never
be per-flow or the fleet multiplies it). Unlimited = the bucket is absent
and the acquire is a no-op (zero overhead on the uncapped path).

### The shell

- `set_rate_cap` command → supervisor → manager; `api.setRateCap`.
- Settings card **Speed**: Unlimited radio + a KB/s numeric field (≥ 1);
  the choice persists (localStorage `ium.rate_cap_kbps`, absent/0 =
  unlimited) and re-applies at boot. Approved card system patterns.
- Copy: the card's sentence states the honest contract — the cap is the
  total across all uploads and lanes.

### Acceptance

- [ ] Bucket unit-tested under virtual time: rate honored (cap 100 B/s ×
      10 s ⇒ ~1000 B granted), fleet SUM ≤ cap with K concurrent acquirers,
      unlimited zero-overhead, swap-rate takes effect on the next acquire.
- [ ] The ladder drill gains the capped rung: cap 50 KB/s on an uncapped
      link → end-to-end measured ≤ cap + small overhead, byte-exact, no
      false evictions (zero eviction events in the run).
- [ ] `set_rate_cap` hot (running jobs obey without restart) + boot
      re-apply pinned.
- [ ] Settings card + copy pins; brand gate walks the new strings.
"""

payload = {"body": body}
n = sys.argv[1] if len(sys.argv) > 1 else "94"
req = urllib.request.Request(
    f"https://api.github.com/repos/kahlanhamza/IUM/issues/{n}/comments",
    data=json.dumps(payload).encode(),
    headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "ium-audit",
    },
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    d = json.loads(resp.read())
print(f"comment id {d['id']}: {d['html_url']}")
