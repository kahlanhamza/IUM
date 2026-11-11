#!/usr/bin/env python3
"""IUM-dist: the first screen answers the provider search (#104).

The public README/USERGUIDE name only "AWS S3, Cloudflare R2, MinIO, Wasabi"
on their S3 rows and never mention Nextcloud. A regular user searching
"Backblaze" or "Nextcloud" finds nothing — the vocabulary wall, public side.

Every replacement below is TRUE FOR THE SHIPPED v0.2.2: the raw-URL S3 lane
reaches every named provider, Nextcloud IS WebDAV, and the v0.2.2 vault
ships the TeraBox / MEGA (rclone bridge) / Feishu lanes (verified at tag
bf594c2). NO picker/share-link wording — that ships in v0.2.3 (#103).

Hygiene gate on the result: zero internal refs (the #102 posture — the gate
covers copy, not just assets), famous names present, tables intact.
"""
import base64
import json
import sys
import urllib.request

REPO = "kahlanhamza/IUM-dist"
TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
}


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        headers={**HDR, **({"Content-Type": "application/json"} if payload else {})},
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        body = r.read()
    return json.loads(body) if body else {}


def replace_once(content: str, old: str, new: str, tag: str) -> str:
    if old not in content:
        assert new in content, f"{tag}: old text gone AND new text absent — aborting"
        print(f"{tag}: already applied, skipping")
        return content
    n = content.count(old)
    assert n == 1, f"{tag}: expected exactly 1 occurrence of the old text, got {n}"
    return content.replace(old, new)


README_EDITS = [
    (
        "hero",
        """  One window, any destination — S3-compatible storage, WebDAV, Google Drive,
  YouTube, Terabox, Feishu, Mega, plain HTTP, TUS — with resume that actually
  resumes, and progress that never lies to you.""",
        """  One window, any destination — Backblaze B2, Cloudflare R2, Wasabi, Scaleway,
  DigitalOcean Spaces, Filebase, AWS S3, MinIO and anything else speaking the
  S3 API; Nextcloud and any WebDAV server; Google Drive, YouTube, Mega,
  TeraBox, Feishu; plain HTTP and TUS — with resume that actually resumes, and
  progress that never lies to you.""",
    ),
    (
        "highlights row",
        "| **Every destination, one form** | S3-compatible buckets (AWS S3, Cloudflare R2, MinIO, Wasabi…), WebDAV, Google Drive, YouTube, Terabox, Feishu, Mega, plain HTTP and TUS servers. Credentials are set once and stored in your OS keychain. |",
        "| **Every destination, one form** | S3-compatible buckets (Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, AWS S3, MinIO — anything speaking the S3 API), Nextcloud and any WebDAV server, Google Drive, YouTube, Mega, TeraBox, Feishu, plain HTTP and TUS. Credentials are set once and stored in your OS keychain. |",
    ),
    (
        "transports S3 row",
        "| S3-compatible | S3 multipart | ✅ per-part | AWS S3, Cloudflare R2, MinIO, Wasabi |",
        "| S3-compatible | S3 multipart | ✅ per-part | AWS S3, Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, MinIO — anything speaking the S3 API |",
    ),
    (
        "transports WebDAV row",
        "| WebDAV | WebDAV `PUT` | bridge-declared | connect with URL + username + password |",
        "| WebDAV | WebDAV `PUT` | bridge-declared | Nextcloud, ownCloud, any WebDAV server — connect with URL + username + password |",
    ),
]

GUIDE_EDITS = [
    (
        "lane list",
        "2. **Give the destination its credentials, once.** Open **Settings** and pick the lane you are using (S3-compatible storage, WebDAV, Google Drive, YouTube, Terabox, Feishu, Mega — each appears once its credentials are configured; lanes without credentials simply do not appear, nothing is half-broken). HTTP and TUS destinations need no credentials.",
        "2. **Give the destination its credentials, once.** Open **Settings** and pick the lane you are using (S3-compatible storage — Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, AWS S3, MinIO and anything else speaking the S3 API; WebDAV — Nextcloud, ownCloud, any WebDAV server; Google Drive, YouTube, Terabox, Feishu, Mega — each appears once its credentials are configured; lanes without credentials simply do not appear, nothing is half-broken). HTTP and TUS destinations need no credentials.",
    ),
    (
        "adapter line",
        "- **Adapter** — the destination type (e.g. `s3_multipart` for any S3-compatible bucket: AWS S3, Cloudflare R2, MinIO, Wasabi…).",
        "- **Adapter** — the destination type (e.g. `s3_multipart` for any S3-compatible bucket: Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, AWS S3, MinIO…).",
    ),
]

FAMOUS = [
    "Backblaze B2", "Cloudflare R2", "Wasabi", "Scaleway",
    "DigitalOcean Spaces", "Filebase", "MinIO", "Nextcloud",
]
FORBIDDEN = [
    "kahlanhamza/IUM/", "provider picker", "share link", "presigned",
    "Closes #", "fixes #", "refs #",
]


def edit_file(path: str, edits: list) -> str:
    cur = api("GET", f"/repos/{REPO}/contents/{path}")
    content = base64.b64decode(cur["content"]).decode()
    changed = False
    for tag, old, new in edits:
        before = content
        content = replace_once(content, old, new, f"{path}:{tag}")
        changed = changed or content != before
    if not changed:
        print(f"{path}: nothing to change")
        return content, None
    put = api("PUT", f"/repos/{REPO}/contents/{path}", {
        "message": (
            "docs: the first screen answers the provider search — the famous "
            "S3-compatible names (B2, R2, Wasabi, Scaleway, DO Spaces, "
            "Filebase) and Nextcloud/ownCloud under WebDAV, all true of the "
            "shipped build"
        ),
        "content": base64.b64encode(content.encode()).decode(),
        "sha": cur["sha"],
    })
    sha = put["commit"]["sha"]
    print(f"{path}: PUT ok, distro commit {sha[:12]}")
    return content, sha


def hygiene(content: str, path: str) -> None:
    low = content.lower()
    for bad in FORBIDDEN:
        assert bad.lower() not in low, f"{path}: internal-ref leak '{bad}'"
    for name in FAMOUS:
        assert name in content, f"{path}: famous provider '{name}' missing"
    for line in content.splitlines():
        if line.startswith("|"):
            assert line.count("|") >= 3, f"{path}: malformed table row: {line[:60]}"
    print(f"{path}: hygiene GREEN (no leaks, {len(FAMOUS)} names present, tables intact)")


def main() -> None:
    readme, rsha = edit_file("README.md", README_EDITS)
    hygiene(readme, "README.md")
    guide, gsha = edit_file("USERGUIDE.md", GUIDE_EDITS)
    hygiene(guide, "USERGUIDE.md")
    print(f"DISTRO COMMITS: README {rsha or '(unchanged)'} | USERGUIDE {gsha or '(unchanged)'}")


if __name__ == "__main__":
    main()
