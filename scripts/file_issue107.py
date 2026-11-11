#!/usr/bin/env python3
"""File #107: the v0.2.3 cut (release engineering per the ratified bar)."""
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()

TITLE = "Cut v0.2.3 — the destination lane ships (screenshots, version sweep, tag choreography, distro sync, unauth battery)"

BODY = """## Design note (why 0.2.3 and why now)

The owner's pre-release bar (2026-11-11): research from reliable sources, every detail correct, both READMEs current, UX/UI verified — then release. #105 closed the accuracy sweep on run 43052820406 8/8; #106 hardened CI on the way. The UX/UI confirmation now stands on: **core 346/346 + vitest 191/191 (a11y incl.)** locally on `e1a3650`, and **two fully-green CI runs** (43052820406, 43054201851) including the WebDriver e2e through the picker's custom seam and the a11y gate.

**Version number, by the ratified doctrine (docs/05 §Versioning policy, #20):** "`0.MINOR` may break; `0.PATCH` is fixes/additions only." The destination lane is pure addition — the Custom URL default stays byte-identical, JobSpec and the adapter contracts are untouched, no compile-fail break exists to pin. → **v0.2.3**.

## The rungs

1. **Screenshots through the real harness, at HEAD.** All six product shots pre-date the picker (re-shot 11-10 11:53; picker landed `e26a7cb` 11-11). Re-shoot with shot 02 driving the GUIDED flow (provider → region → bucket → derived URL) so the release's headline is visible; the harness's engine sentence says v0.2.1 and moves to v0.2.3 with the sweep.
2. **CHANGELOG [0.2.3]** — the [Unreleased] destination-lane entry (already issue-cited) becomes the release section; the #105 refinements fold in (guided region reaches the signing scope; the registry corrected against the providers' own 2026 docs: Filebase `.io`, Wasabi 16 regions, DO 13, B2 +us-east-005/−us-west-003).
3. **Version sweep** — workspace + member crates + package.json + tauri.conf + every test fixture + the harness sentence, 0.2.2 → 0.2.3, one commit, all gates on the committed tree.
4. **Tag choreography** — annotated tag `v0.2.3` on the swept HEAD, push, release workflow run chased to green (builds all installers + signatures + SHA256SUMS + latest.json, publishes cross-repo to IUM-dist).
5. **The unauth battery** — feed URL resolves with version 0.2.3 and platform keys = shipped hardware; signatures byte-match the published `.sig` assets; installer checksums match `SHA256SUMS.txt`; the release is the distro's "latest".
6. **Distro surface at the cut** — README/USERGUIDE/CHANGELOG on IUM-dist move to v0.2.3-true wording (the picker/share-link claims become downloadable truth per the #104 scope-honesty rule) + the re-shot assets; unauth-verified.
7. **STATUS + closure** — the row, the receipts, the release issue closed on evidence.

## Acceptance criteria

- [ ] Six fresh shots through the harness, shot 02 shows the guided picker; READMEs reference current files
- [ ] CHANGELOG [0.2.3] cites #103/#105/#106; version sweep lands as one commit with gates green
- [ ] Tag pushed; release run green; every installer + sig + sums published
- [ ] Unauth battery green (feed, signatures, checksums, latest)
- [ ] Distro README/USERGUIDE/CHANGELOG v0.2.3-true, unauth-verified, zero internal refs
- [ ] STATUS row + issue closed with run IDs
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": TITLE, "body": BODY, "labels": ["release"]}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"filed #{out['number']}: {out['html_url']}")
