#!/usr/bin/env python3
"""#107 closure: STATUS row + the release evidence comment."""
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()

COMMENT = """## GREEN evidence — v0.2.3 is live and byte-verified from the user's chair

**Release run 43062849275: SUCCESS** (all three installer jobs 20/20 + publish 18/18) on the tag HEAD `659aa2c`; **CI run 43062808005: SUCCESS** on the same tree. The battery:

- **Unauth battery 24/24 ALL GREEN** (`scripts/verify_distro_release_v023.py`): `latest.json` resolves anonymously with version 0.2.3; platform keys are exactly the shipped hardware (`darwin-aarch64`, `linux-x86_64`, `windows-x86_64`); each manifest signature byte-matches the published `.sig` asset; **all six installers sha256-verified against `SHA256SUMS.txt` as real downloaded bytes** (~120 MB fetched); the release is the distro's "latest", not draft; **15 assets, zero provenance-shaped files** (the #102 posture holds).
- **Distro surface, unauth-verified**: README carries the picker paragraph + share-link claim + picker-worded S3 row; USERGUIDE teaches the picker flow, the Test button and the share link, with v0.2.3 examples; CHANGELOG has the [0.2.3] section — **zero issue numbers, zero ADR refs, 8 famous names present** (hygiene asserted in `scripts/distro_sync_v023.py`, idempotent, byte-diffed before any PUT).
- **The screenshots are the release's headline**: all six re-shot through the real harness at HEAD (shot 02 drives the GUIDED flow — Backblaze B2 preset, region select, bucket, derived URL, keys link; the status bar reads v0.2.3); distro assets updated.
- **The honest detour on the record**: the tag first rode `ff6a62f`, whose release.yml carried the one apt step the #106 sweep didn't cover — the ubuntu installer job wedged 26+ min on the azure mirror stall while windows/macOS went 20/20. With nothing published yet, the pre-release tag move applied: release.yml hardened (both apt sites), tag re-cut at `659aa2c`, run restarted — **GREEN first try, and the hardened step passed in minutes**.

Version number per the ratified doctrine (docs/05 §Versioning policy, #20): `0.PATCH` is fixes/additions only — the destination lane is pure addition → **0.2.3**."""

ROW = (
    "| 2026-11-11 (the v0.2.3 cut, Closes #107) "
    "| **The destination lane ships.** Version by the ratified doctrine (0.PATCH = fixes/additions; the lane is pure addition). "
    "RUNGS: (1) all six product shots re-shot through the real harness at HEAD — shot 02 now drives the GUIDED picker flow (B2 preset, region, bucket, derived URL, keys link; status bar v0.2.3); (2) CHANGELOG [0.2.3] with the issue-cited entries; (3) the version sweep — workspace + shell crate + tauri.conf + package.json + BOTH lockfiles + the two doc comments + the harness sentence, one commit `ff6a62f`, full gate GREEN on the committed tree (after the target dir was wiped and re-warmed: 1658 dep files had accumulated past the 9.9G disk again — the gate needs a clean-target+prewarm recipe at this session length); "
    "(4) the tag: first cut rode the unhardened release.yml — the ubuntu installer job wedged 26+ min on the azure stall (windows/macOS 20/20) — with NOTHING published the pre-release tag move applied: release.yml hardened (both apt sites: deps + xvfb smoke), tag re-cut at `659aa2c`, **release run 43062849275 SUCCESS first try** (installer jobs 20/20 ×3 + publish 18/18), CI 43062808005 success; "
    "(5) the unauth battery 24/24 (`verify_distro_release_v023.py`): feed 200 unauth @0.2.3, platform keys = shipped hardware, manifest sigs == .sig bytes, **all six installers sha256-verified as real bytes**, latest-not-draft, 15 assets zero provenance; "
    "(6) the distro surface synced (README picker paragraph + share-link claim; USERGUIDE picker/Test/share-link flow + v0.2.3 examples; CHANGELOG [0.2.3]; six re-shot assets) — hygiene-asserted (no issue numbers, no ADRs, 8 famous names), idempotent, unauth-verified; "
    "(7) this row. LESSON: the #106 mirror hardening must sweep EVERY workflow with an apt step — release.yml was the one that bit, during a cut. "
    "| release run 43062849275 green; battery 24/24; distro unauth-clean | ff6a62f + 659aa2c (+ distro commits) |"
)

with open("STATUS.md", encoding="utf-8") as f:
    content = f.read()

marker = "|---|---|---|---|\n"
hdr_idx = content.index("## 9. Session log")
sep_idx = content.index(marker, hdr_idx) + len(marker)
content = content[:sep_idx] + ROW + "\n" + content[sep_idx:]

content = content.replace(
    "| #105 + #106 closed on run 43052820406 8/8; the release lane is next (screenshot re-shoot, distro v0.2.3 wording, the version sweep) | this commit)**",
    "| v0.2.3 IS PUBLISHED (release run 43062849275; battery 24/24; distro surface v0.2.3-true and unauth-verified) | this commit)**",
)

with open("STATUS.md", "w", encoding="utf-8") as f:
    f.write(content)
print("STATUS row + header finalized")

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/107/comments",
    data=json.dumps({"body": COMMENT}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"#107 evidence: {out['html_url']}")
