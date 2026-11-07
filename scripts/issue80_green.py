#!/usr/bin/env python3
"""GREEN evidence comment on #80."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDRS = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}

body = """## GREEN — landed on `main` as `f29336e` (Closes #80)

**Runtime:** both plugins registered, capabilities granted, `src/updater.ts` wraps the
whole story — check automatic, install user-armed, per-chunk deltas normalized to
cumulative progress, honest degrade outside Tauri. Product surface: the About card
(six states, errors as sentences) + the silent 4 s startup check whose failures stay
silent (the update-available sentence joined the enumerated `APP_COPY` surface).

**Pipeline:** `release_collect.sh` now globs the updater bundles + `.sig` sidecars;
`scripts/build_latest_json.py` builds the tauri v2 manifest with the pairing rule
enforced (unsigned bundle = build refusal, self-test proven); `release.yml` builds
`latest.json` before the attest step so the manifest rides the SLSA subjects;
`ci.yml` gained the manifest self-test lane (the #72 cannot-rot pattern).

**Gates:** vitest **75/75** (69+6) · tsc strict · vite build 195.64 KB · fmt clean ·
`Cargo.lock` resolved (+326 lines) · manifest self-test PASS · both workflows
YAML-parse · `gate.sh` GREEN on the committed tree `f29336e` · **CI success** incl.
the Rust shell lane compiling the two plugin crates.

**Honest remaining step:** the real updater handshake (v0.1.2 → next tag e2e on a
real machine) proves itself at the next release rehearsal — the endpoint starts
serving `latest.json` from the next tag onward."""

data = json.dumps({"body": body}).encode()
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/80/comments",
    data=data, headers=HDRS, method="POST")
print(json.load(urllib.request.urlopen(req))["html_url"])
