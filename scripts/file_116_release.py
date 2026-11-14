#!/usr/bin/env python3
"""File #116 — the v0.2.5 cut (the quality-recovery release)."""
import json
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()

BODY = """## The cut

v0.2.5 — the quality-recovery release: the owner's v0.2.4 defect report (the big-tech bar, #66) shipped as three fixes this session, plus the first-launch boot race from the previous one. Every fix is already landed, RED-first, gate-green on its own commit:

- **#112** — the first-launch boot race (the "engine unreachable" screenshot: Tauri builds windows before `.setup()` manages the engine; the UI's one-shot probes pinned the transient forever). Fixed by the bounded-retry boot contract (`boot.ts`).
- **#113** — the connect sheets rendered one-on-top-of-another: create-form 4-column grid + nowrap labels. Fixed by `.sheet-grid` (one field per row), wrapping labels, and the S3 endpoint label cut to one line.
- **#114** — the settings dialog went translucent under Windows 11 Mica (app content + wallpaper printing through the modal). Fixed by `--surface-solid`: floating surfaces are opaque layers, chrome keeps the material.
- **#115** — connect opened a cmd window and handed the browser a URL mangled by cmd's `&` parsing (the live AADSTS900144 wire-fact). Fixed by the explorer.exe handoff (URL as one argv, byte-verbatim) + `CREATE_NO_WINDOW` on every Windows spawn (rclone authorize, the boot probe, all five bridge spawns).

## Rungs (the #111 shape, repeated)

1. [ ] CI green on the release-head SHA (all fixes included).
2. [ ] Version sweep 0.2.5: workspace + shell crate + tauri.conf + package.json + both locks; the updater mock honestly newer-than-shipped (0.2.6); CHANGELOG [0.2.5] issue-cited; README/USERGUIDE truth.
3. [ ] Product shots re-shot through the real harness (the settings sheet CHANGED this release — the connect-sheet shot must show the new one-field-per-row layout), version verified ON the rendered UI by the playwright probe.
4. [ ] gate GREEN on the committed release tree.
5. [ ] Tag v0.2.5 cut ONCE on the verdict SHA (the published-tag-move lesson stands).
6. [ ] Release run SUCCESS (mac dmg/app, ubuntu deb/appimage/rpm, windows msi/nsis, publish).
7. [ ] Unauthenticated battery ALL GREEN (latest.json 0.2.5, platform keys, sigs byte-identical, sha256-exact installers, no provenance files).
8. [ ] IUM-dist surface synced + live-verified (README, USERGUIDE, sanitized CHANGELOG, new shots — hygiene-asserted).

Auto-update: v0.2.2+ installs take v0.2.5 through the updater lane — the first-launch boot race and every defect in the owner's report die on update, no reinstall.
"""

req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/issues",
    data=json.dumps({"title": "v0.2.5 — the quality-recovery cut (#112 boot race + #113 sheets + #114 modal + #115 OAuth handoff)", "body": BODY, "labels": ["release", "product", "release-blocker"]}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req) as r:
    n = json.load(r)["number"]
print(f"#{n} created")
# milestone 10
req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/issues/{n}",
    data=json.dumps({"milestone": 10}).encode(),
    method="PATCH",
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req):
    print("milestone set")
