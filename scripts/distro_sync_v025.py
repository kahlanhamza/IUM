#!/usr/bin/env python3
"""IUM-dist v0.2.5 surface sync (#116 rung 8): README/USERGUIDE/CHANGELOG
move to v0.2.5-true wording — the quality-recovery release (the connect
sheet renders one field per row, the dialog stays opaque, sign-in never
opens a console window) — and the seven re-shot product assets land,
including the NEW connect-sheet shot (the exact surface the owner's
v0.2.4 screenshot showed broken). Every edit is asserted: product-only
language (no issue numbers, no ADRs, no internal paths), tables intact.
"""
import base64
import json
import os
import re
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()
REPO = "kahlanhamza/IUM-dist"
HDR = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
       "User-Agent": "ium-release"}


def api(method, path, payload=None, expect=200):
    req = urllib.request.Request(f"https://api.github.com/repos/{REPO}/{path}",
                                 method=method, headers=HDR,
                                 data=json.dumps(payload).encode() if payload is not None else None)
    with urllib.request.urlopen(req) as r:
        body = json.load(r) if r.status != 204 else {}
        assert r.status == expect or (method == "PUT" and r.status in (200, 201)), \
            f"{method} {path}: {r.status} != {expect}"
        return body


def get_file(path):
    r = api("GET", f"contents/{path}")
    return base64.b64decode(r["content"]).decode(), r["sha"]


def put_file(path, content, sha, message):
    api("PUT", f"contents/{path}", {
        "message": message,
        "content": base64.b64encode(content.encode()).decode(),
        "sha": sha,
    })


def put_binary(path, data, sha, message):
    payload = {"message": message, "content": base64.b64encode(data).decode()}
    if sha:
        payload["sha"] = sha
    api("PUT", f"contents/{path}", payload)


def hygiene(name, text, famous=True):
    assert not re.search(r"#\d{2,3}", text), f"{name}: issue number leaked"
    assert "ADR-" not in text, f"{name}: ADR leaked"
    assert ".secrets" not in text and "/home/z" not in text and "ium-core" not in text \
        and "ium-app" not in text and "cmd /c start" not in text, f"{name}: internal path leaked"
    if famous:
        for f in ("Backblaze", "Cloudflare", "Wasabi", "Nextcloud"):
            assert f in text, f"{name}: {f} missing"
    assert "【" not in text and "TODO" not in text, f"{name}: placeholder leaked"


# ---------------------------------------------------------------- assets
SHOTS = [
    "01-main-window.png", "02-create-upload-filled.png", "03-inspector-chunks.png",
    "04-settings-privacy.png", "05-settings-credentials.png", "06-main-window-light.png",
    "07-connect-sheet-s3.png",
]
LOCAL = "/home/z/my-project/docs/screenshots"
existing = {x["name"]: x["sha"] for x in api("GET", "contents/assets")}
commit_note = "the v0.2.5 re-shot surface — status bar v0.2.5, the fixed connect sheet"
for name in SHOTS:
    data = open(f"{LOCAL}/{name}", "rb").read()
    put_binary(f"assets/{name}", data, existing.get(name), commit_note)
    print(f"assets/{name} uploaded ({len(data)} bytes)")
hygiene("assets listing", " ".join(SHOTS), famous=False)

# ---------------------------------------------------------------- README
readme, sha = get_file("README.md")

old_last_row = "| ![Settings — the credentials vault and About](assets/05-settings-credentials.png) | ![The light scheme — the logo ink on paper white](assets/06-main-window-light.png) |"
new_last_row = old_last_row + "\n| ![The S3 connect sheet — one field per row, labels that wrap](assets/07-connect-sheet-s3.png) | |"
if "assets/07-connect-sheet-s3.png" not in readme:
    assert old_last_row in readme, "README: the shots table drifted"
    readme = readme.replace(old_last_row, new_last_row)

hygiene("README.md", readme)
remote, _ = get_file("README.md")
if remote != readme:
    put_file("README.md", readme, sha, "README: the connect-sheet shot — one field per row, labels that wrap (the v0.2.5 surface)")
    print("README.md synced")
else:
    print("README.md already true")

# ---------------------------------------------------------------- USERGUIDE
guide, sha = get_file("USERGUIDE.md")

guide = guide.replace(
    "for example, v0.2.4 shipped `IUM_0.2.4_amd64.deb`, `IUM-0.2.4-1.x86_64.rpm`, `IUM_0.2.4_amd64.AppImage`, `IUM_0.2.4_aarch64.dmg`, `IUM_0.2.4_x64_en-US.msi`, and `IUM_0.2.4_x64-setup.exe`",
    "for example, v0.2.5 shipped `IUM_0.2.5_amd64.deb`, `IUM-0.2.5-1.x86_64.rpm`, `IUM_0.2.5_amd64.AppImage`, `IUM_0.2.5_aarch64.dmg`, `IUM_0.2.5_x64_en-US.msi`, and `IUM_0.2.5_x64-setup.exe`",
)
guide = guide.replace("certutil -hashfile IUM_0.2.4_x64-setup.exe SHA256",
                      "certutil -hashfile IUM_0.2.5_x64-setup.exe SHA256")
assert "IUM_0.2.4" not in guide, "USERGUIDE: a 0.2.4 example survived"

hygiene("USERGUIDE.md", guide)
remote, _ = get_file("USERGUIDE.md")
if remote != guide:
    put_file("USERGUIDE.md", guide, sha, "USERGUIDE: the v0.2.5 installer examples")
    print("USERGUIDE.md synced")
else:
    print("USERGUIDE.md already true")

# ---------------------------------------------------------------- CHANGELOG
log, sha = get_file("CHANGELOG.md")

entry = """## [0.2.5] — 2026-11-14

### Fixed
- **The connect sheets render one field per row**: Settings → connections
  no longer crams several credential fields side by side; every field sits
  on its own row, labels wrap instead of painting over each other, and the
  S3 bucket-URL field is one clear line of copy (the example lives in the
  note above it).
- **The settings dialog and command palette stay opaque under Windows 11
  Mica**: the dialog no longer prints the app content and wallpaper
  through itself; what you read sits on a solid layer.
- **Connect no longer opens a console window or a broken sign-in page**:
  the sign-in page opens in your browser with the full request attached —
  the scope Microsoft was refusing to see is back — and every background
  helper hides its console window.
- **First-launch reliability**: a cold first boot no longer strands the
  app on "engine unreachable"; the interface retries within the boot
  window and recovers on its own, so even a slow 30-second cold start
  reaches a working window without a restart.

"""
if "## [0.2.5]" not in log:
    marker = "## [0.2.4]"
    assert marker in log, "CHANGELOG: the 0.2.4 heading drifted"
    log = log.replace(marker, entry + marker, 1)

hygiene("CHANGELOG.md", log, famous=False)
remote, _ = get_file("CHANGELOG.md")
if remote != log:
    put_file("CHANGELOG.md", log, sha, "CHANGELOG: the sanitized [0.2.5] entry")
    print("CHANGELOG.md synced")
else:
    print("CHANGELOG.md already true")

print("SYNC COMPLETE")
