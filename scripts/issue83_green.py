#!/usr/bin/env python3
"""GREEN evidence + CI verdict for #83 (WebDAV) — commit 50e3765, gate.sh GREEN, CI success."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**GREEN** — landed at 50e3765 (`feat(app,core): the WebDAV lane — url + username + password, Test connection, hot apply`), `Closes #83` in the body (closed by the push).

**Gates, all local on the committed tree:**
- `cargo test -p ium-core` — **288 green** (+4 webdav: vendor normalization, the required-credential refusals naming the missing env var, the env-contract builder `RCLONE_CONFIG_IUM_WEBDAV_{TYPE,URL,USER,PASS[,VENDOR]}`, the empty-vendor omission); live drills skip loud (no credentials in CI).
- `cargo fmt --all -- --check` clean · `cargo clippy -p ium-core --all-targets -- -D warnings` clean.
- vitest **93/93** · tsc strict · vite build 201.63 KB.
- `bash scripts/gate.sh` — **GREEN on the committed tree 50e3765** (fmt + clippy + full test suite + doc lane + frontend lane, worktree checkout — what was pushed is what was tested).

**CI:** run on 50e3765 — **completed / success** (REST verdict, no cron).

**What a user sees:** Settings → **WebDAV** row → Connect opens the focused sheet (server URL, username, password / app password, optional vendor `nextcloud|owncloud|sharepoint`, target folder) → **Test** proves the credentials with a read-only probe → **Save & connect** hot-applies the lane — no restart. The password rides the OS keychain (#81); the env remote is `IUM_WEBDAV` (never shadows a user's own `webdav` remote); the upload path is the proven rclone bridge (#63), resumability honest-by-capability.

Next in the owner's ruling: #84 (Google Drive — one-click Connect) and #85 (YouTube — native resumable adapter + Google OAuth).
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/83/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted GREEN evidence: {out['html_url']}")
