#!/usr/bin/env python3
"""File the v0.2.4 release issue — the native connectors cut (#109 + #110)."""
import json
import urllib.request

BODY = """## The v0.2.4 cut — the native connectors release

**Scope (the v0.2.3..HEAD delta that ships):** the two provider-native transfer lanes — **#109 native Dropbox** (`273b073`, live-rehearsed on the real API) and **#110 native OneDrive** (`06968b0` + the live rehearsal GREEN, CI 43255641309 / 43601962777). Pure additions → **0.PATCH bump: 0.2.3 → 0.2.4** per the ratified versioning doctrine.

**The rungs (the #107 pattern, the #65 release bar + #66 product bar standing rules apply):**

1. Product shots re-shot through the real harness at HEAD (the status bar speaks the release version).
2. CHANGELOG `[0.2.4]` — entries cite #109/#110; evidence lives in the tracker.
3. The version sweep, one commit: workspace `Cargo.toml` + `ium-app/src-tauri/Cargo.toml` + `tauri.conf.json` + `ium-app/package.json` + BOTH lockfiles + the two doc comments (`readme.test.ts`, `logic.ts`) + the harness `ENGINE_SENTENCE` (×2) + the updater mock honestly newer-than-installed.
4. **Gates on the committed tree** (non-bypassable, ADR-007) → push → CI 8/8.
5. Tag `v0.2.4` on the verdict SHA → release run (release.yml carries the v0.2.3 apt-hardening) → SUCCESS required BEFORE anything publishes; nothing may need a tag move afterward (the v0.2.2 lesson: a moved tag flips a published release to DRAFT).
6. The unauth battery (the `verify_distro_release_v023.py` pattern adapted): feed 200 unauth at 0.2.4, platform keys = shipped hardware, manifest sigs == `.sig` bytes, installer sha256s exact against SHA256SUMS.txt, latest-not-draft, zero provenance assets.
7. Distro surface sync (README / USERGUIDE / CHANGELOG on IUM-dist, hygiene-asserted: no issue numbers, no internals) + unauth verify.
8. STATUS row → close this issue with the receipt.

**Pre-flight audit (done at filing):** release range pickaxe-clean (no token shapes); the two UUID auto-snapshot commits in range are rehearsal-driver iterations only; `.secrets/` wiped by a recycle and restored (`gh_token` 10th restore, this value paste count 1 → rotation at the next break stays standing); the local `DISTRO_TOKEN` copy died with the wipe — rung 7 uses the API where the fine-grained token reaches, otherwise the block is recorded honestly and does not block rungs 1–6 or the source-repo release.

**Definition of done:** every rung verified on the record, the release not draft, the feed serving 0.2.4 to the shipped hardware, this issue closed completed.
"""

tok = open("/home/z/my-project/.secrets/gh_token").read().strip()
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({
        "title": "The v0.2.4 cut — the native connectors release (#109 Dropbox + #110 OneDrive)",
        "body": BODY,
        "labels": ["release"],
    }).encode(),
    headers={"Authorization": f"Bearer {tok}", "Accept": "application/vnd.github+json"},
    method="POST",
)
out = json.load(urllib.request.urlopen(req))
print(f"issue #{out['number']} filed: {out['html_url']}")
