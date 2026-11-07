#!/usr/bin/env python3
"""File #80: the auto-update lane — wire the updater runtime + publish its artifacts."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDRS = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}

body = """## What

#65's release bar says **auto-update default-on**. The v0.1.1 pipeline ships *signed-capable*
config — `createUpdaterArtifacts: true`, the rotated minisign key rides the release env
(#71), `tauri.conf.json` pins the endpoint `releases/latest/download/latest.json` — and then
stops. Two hard gaps, measured today:

1. **The runtime never checks.** No `tauri-plugin-updater` / `tauri-plugin-process` in
   `src-tauri/Cargo.toml`, no registration in `lib.rs` (only `dialog`), no capability
   permission, no npm packages, no UI. The plugin block in `tauri.conf.json` is dead config.
2. **The artifacts never ship.** v0.1.1's 17 release assets contain **zero** `.sig` files and
   **no** `latest.json` — `release_collect.sh` globs only `*.deb/*.rpm/*.AppImage/*.dmg/*.msi/*.exe`.
   The endpoint serves a 404 today.

## Design note

**Runtime (the shell):**
- Deps: `tauri-plugin-updater` + `tauri-plugin-process` (Rust), `@tauri-apps/plugin-updater`
  + `@tauri-apps/plugin-process` (npm). Register both in `lib.rs`; grant
  `updater:default` + `process:allow-restart` in `capabilities/default.json` (the #40 lesson:
  capability files grant NOTHING by default — write the permission down or the IPC is silently
  denied).
- `src/updater.ts`: the guarded wrapper (#67 pattern — degrade, never blank, outside Tauri).
  `checkForUpdate()`, `downloadAndInstall(onProgress)`, `restartToUpdate()`.
- Product surface (big-tech pattern, calm voice per the brand sheet):
  - **startup**: ONE silent check ~4 s after boot, Tauri-only, never blocks, never toasts on
    "up to date"; an available update raises a quiet card in the header area —
    "IUM vX is available. Restart to update." + Later.
  - **Settings → About**: a manual "Check for updates" control with honest states —
    Checking… / Up to date (vX) / vX available → Downloading {pct}% → Restart to update.
    Errors surface as a sentence, not a silence.
- No auto-install without consent in this lane: check = automatic, install = user-armed.
  (Sparkle/Squirrel school: silent check, explicit apply.)

**Release pipeline (the payload):**
- `release_collect.sh` additionally globs the updater bundles + signatures the tauri build
  emits (`*.msi.zip`+`.sig`, `*.nsis.zip`+`.sig`, `*.AppImage.tar.gz`+`.sig`,
  `*.app.tar.gz`+`.sig`) and sha256 sidecars them like the installers.
- NEW `scripts/build_latest_json.py` (stdlib-only, `--self-test`): consumes the collected
  updater artifacts + the version, emits `latest.json` (tauri v2 schema: version, notes,
  pub_date, platforms{darwin-aarch64/darwin-x86_64/linux-x86_64/windows-x86_64 → signature,
  url}) — the release publish job uploads it among the assets. Self-test proves the schema
  shape + the sig-pairing rule (no artifact without its .sig sidecar — an unsigned entry is
  a shipped brick, fail loud).
- release.yml publish job: include the new files in the asset upload (check the existing
  glob list).

**Gates:**
- RED first: vitest contracts on `src/updater.ts` (missing module at HEAD) + the About
  control's state machine (mocked api) + the honest non-Tauri degrade.
- Local: vitest + tsc + vite build + fmt/clippy/core suite (core untouched) + `--self-test`
  of the JSON builder + YAML parse of release.yml.
- Rust shell compile + the real updater handshake = CI (the #61 standing contract; budget
  the usual red-run risk when src-tauri is touched). The END-TO-END update path
  (v0.1.2 → v0.1.3 handshake) is proven at the next release rehearsal, on the record.

**Non-goals:** no update-server (the GitHub releases endpoint is the channel); no forced
restart; no differential/delta downloads (tauri handles full bundles); Linux deb/rpm ride the
AppImage channel for updates — honest note in the About copy is not needed (the updater only
sees what latest.json declares).

Refs: #65 (the release bar), #71 (the signing key), #40 (the capability lesson), #77 (About).
"""

data = json.dumps({"title": "Auto-update: wire the updater runtime + publish its artifacts (release-bar lane)", "body": body}).encode()
req = urllib.request.Request("https://api.github.com/repos/kahlanhamza/IUM/issues", data=data, headers=HDRS, method="POST")
issue = json.load(urllib.request.urlopen(req))
print("filed:", issue["number"], issue["html_url"])
