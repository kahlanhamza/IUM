#!/usr/bin/env python3
"""File issue #94: Sprint-2 UX — the speed-cap control (core + shell)."""
import json
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """## Design note (Sprint-2 UX lane 2 — sketch; the full note lands when the lane opens)

The second real Sprint-2 gap (roadmap item 32a). The user's machine shares its
uplink with everything else; the engine today always takes what the link gives
(the prober measures the link, it never POLITES it). The feature: a **global
upload cap** the user sets once and trusts.

### Inventory / constraints (wire facts)

- No rate limiter exists in `ium-core` today (the only `rate limit` mentions
  are comments; the throttle ladder is harness-side).
- The manager already has the policy seam: `set_policy` / `set_eviction_policy`
  / `set_resplit_policy` — a `set_rate_cap` rides the same shape.
- Enforcement point: the chunk drive loop, BEFORE each adapter write — a
  shared token-bucket (bytes budget refilled at `cap B/s`) acquired by every
  worker, so the cap is the SUM across parallel flows (the honest vocabulary
  from #46: nothing beats the bearer sum — the cap divides it, it must not be
  per-flow or the fleet multiplies the cap).
- Cap 0 is not a state ("paused" already exists); the UI offers Unlimited +
  a KB/s field. Changes apply to RUNNING jobs (the bucket is global, the next
  acquire simply sees the new rate) and are journaled (the journal is the only
  truth).
- The shell: Settings card **Speed** (Unlimited radio + numeric field), wired
  through a `set_rate_cap` command → manager; the card follows the approved
  card system; the full mockup note lands when the lane opens (approve-first
  law).

### Acceptance (draft)

- [ ] Token-bucket in core, unit-tested (rate honored over mock time; fleet
      SUM never exceeds the cap; unlimited = zero overhead).
- [ ] `set_rate_cap` journaled + hot (running jobs obey without restart).
- [ ] Settings card + command + copy pins.
- [ ] The throttled ladder drill (#45) gains one capped rung: cap 50 KB/s on
      an uncapped link → measured throughput ≤ cap + small overhead.
"""

payload = {
    "title": "Sprint-2 UX: the speed-cap control — a global, journaled upload cap (core + shell)",
    "body": body,
    "labels": [],
}
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
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
print(f"filed #{d['number']}: {d['title']}")
