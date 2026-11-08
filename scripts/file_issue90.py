#!/usr/bin/env python3
"""File follow-up issue #90: same curl -f hardening for the remaining fetch sites."""
import json
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """Follow-up sweep from #89 (the nightly drill bootstrap fix). The repo-wide grep for
`curl -s…L` fetch sites found the same no-`--fail` class outside the two drill scripts.
One defect class, one issue, so #89 stays minimal.

Sites (wire facts, HEAD):

| File | Lines | Pattern |
|---|---|---|
| `scripts/issue50_wire_rehearsal.sh` | 54–55 | `curl -sSL --retry 3 -o …` + sha256 guard — same poisoning risk |
| `scripts/issue55_wire_rehearsal.sh` | 68–69 | same |
| `.github/workflows/release.yml` | 225–226 | `curl -sSLO` syft + checksums (release-time, rarer) |
| `.github/workflows/stress.yml` | 63–66 | `curl -sL --retry 3 -o` sample media (no checksum guard at all) |
| `scripts/dbg-s3-iso.sh` | 21 | debug fetch (low stakes) |
| `scripts/release_supply_chain_check.sh` | 104 | API asset fetch with bearer token |

Acceptance:
- [ ] `-f` + `--retry-delay 2` on every artifact fetch above.
- [ ] digest-format guards where a checksum file is compared.
- [ ] stress.yml: decide explicitly whether sample media needs a pinned checksum
      (it is a fixed Big Buck Bunny/tears-of-steel URL — pinning is cheap and honest).
- [ ] One commit, gates, push.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps(
        {
            "title": "curl hardening sweep: -f + digest guards for the remaining artifact fetch sites (#89 follow-up)",
            "body": body,
            "labels": ["ci", "reliability"],
        }
    ).encode(),
    headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
    },
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    out = json.load(resp)
print(f"filed issue #{out['number']} url={out['html_url']}")
