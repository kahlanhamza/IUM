#!/usr/bin/env python3
"""File #106: ubuntu CI jobs wedge on the azure.archive.ubuntu.com stall."""
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()

TITLE = "CI infra: the ubuntu jobs wedge on the azure.archive.ubuntu.com mirror stall — harden the apt steps (canonical mirror + retries)"

BODY = """## Wire facts (run 43036333957, HEAD `aa745b0` — the #105 verdict chase)

- Attempt 1: six of eight jobs GREEN (fmt 6/6, core ×3 OS 18/18 steps, ium-app windows/macos 14/14). Both **ubuntu** jobs (`ium-app ubuntu`, `app-e2e`) sat 35+ minutes on the apt step with zero progress.
- The live log shows the stall: apt `Ign:`-ing **every** package from `http://azure.archive.ubuntu.com/ubuntu` at exactly 20-second intervals — a dead/stalled default mirror, not a code problem (the same step passed green in run 43008204325 the day before; this lane touches only TS/JSON/markdown).
- Attempt 2 (re-run-failed-jobs) wedged at the same step, same signature. Cancelled; log pulled and read before any change.

## Design note (the hardening)

The two ubuntu apt steps run raw `apt-get update` + `install` against the image's default Azure mirror with no retries and no fallback. Harden both:

1. Rewrite `azure.archive.ubuntu.com` → `archive.ubuntu.com` (the canonical, CDN-fronted mirror) in the image's deb822 sources before apt runs — deterministic, removes the stall cause.
2. `-o Acquire::Retries=5 -o Acquire::http::Timeout=15` on both update and install — a transient blip retries instead of crawling at 20 s per package.

## Acceptance criteria

- [ ] Both ubuntu jobs pass the deps step and the full run is 8/8 GREEN on the HEAD that carries #105 + this fix
- [ ] The incident (signature: `Ign:` lines every 20 s from azure.archive) is recorded in the STATUS row
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": TITLE, "body": BODY, "labels": ["ci"]}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"filed #{out['number']}: {out['html_url']}")
