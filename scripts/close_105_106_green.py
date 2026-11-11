#!/usr/bin/env python3
"""Post GREEN evidence on #105 and #106 (run 43052820406)."""
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()

B105 = """## GREEN evidence (closed by the keyword; receipts here)

**CI run 43052820406 — 8/8 SUCCESS** on `2058327` (carries the three fix commits `d80655a` + `c31c4fe` + `e1a3650` plus the #106 CI hardening): fmt 6/6 · core ×3 OS 18/18 steps · ium-app ×3 OS 14/14 · app-e2e (the WebDriver lane through the picker's custom seam) green.

**Local gate, full battery on the committed tree `e1a3650`**: fmt + clippy `-D warnings` + **core 346/346** (incl. trybuild contract + the `providers_registry` drift guard **7/7** over the edited JSON — every new sample URL re-proven through the REAL `parse_target`) + doc lint + tsc strict + vite + **vitest 191/191** (a11y suite included; the two new #105 pins green: guided region write-through before `createJob`, custom mode writes nothing).

Acceptance: every checkbox met — RED→GREEN pinned, registry matches the providers' own docs (Filebase `.io`, Wasabi 16, DO 13, B2 +us-east-005/−us-west-003), docs say what actually ships. Honesty notes stand in the STATUS row: B2's CA East code still uncorroborated (follow-up), screenshots re-shoot + distro v0.2.3 wording land at the cut."""

B106 = """## GREEN evidence + the incident record

**CI run 43052820406 — 8/8 SUCCESS** on `2058327`. Both ubuntu jobs passed the hardened deps step.

The three-run diagnosis trail, for the record:
1. `43036333957` (attempts 1-2): azure.archive.ubuntu.com stalled — `Ign:` every package at 20 s intervals, 35+ min wedges. Six non-apt jobs green both times.
2. `43044589016`: the first hardening (sed over ubuntu.sources/sources.list) **no-opped** — the log's `Get:1 file:/etc/apt/apt-mirrors.txt Mirrorlist` proved GitHub's noble image routes apt through a LOCAL MIRRORLIST file that still carried azure. Same stall.
3. `43049461021`: mirrorlist sed **worked** (Ign lines said archive.ubuntu.com) — and the canonical mirror stalled IDENTICALLY (18+ min vs the **44 s baseline** measured on yesterday's green run 43008204325). HTTPS endpoints on the same runner fetched in seconds; all archives answered 200 from outside. Conclusion: the runner's **HTTP route** to the Ubuntu archives was dead, not Ubuntu.
4. Fix: apt over **HTTPS to mirrors.edge.kernel.org** (Fastly-CDN; scheme+host rewrite across mirrorlist + deb822 + classic sources). **GREEN on the first try.**

The hardening stays: if a mirror ever stalls again, the runner no longer crawls it — it times out at 15 s, retries 5×, and the canonical-CDN route is scheme-proof."""

for n, body in ((105, B105), (106, B106)):
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/issues/{n}/comments",
        data=json.dumps({"body": body}).encode(),
        headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(req) as r:
        out = json.load(r)
    print(f"#{n} GREEN posted: {out['html_url']}")
