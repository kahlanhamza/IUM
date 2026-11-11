#!/usr/bin/env python3
"""#109 credential-block record correction (2026-11-12).

Three corrections to the lane record:
1. Scope trim is IMPOSSIBLE — Dropbox enforces `account_info.read` on every
   scoped app and auto-adds `files.metadata.read` with any `files.content.*`
   scope. Owner verified in the console: neither checkbox can be cleared.
   Final scope set (accepted, GO unchanged):
     account_info.read, files.content.read, files.content.write, files.metadata.read
2. Allow public clients: owner confirmed ON in the app console → every console
   prerequisite is complete → credential block FULLY LIFTED.
3. Redirect URI plan (loop #105: no unverified values registered): the official
   OAuth guide requires "the exact redirect URI(s) ... registered under
   Redirect URIs"; port is part of the exact match (community-confirmed by
   Dropbox staff). Engine therefore uses one FIXED loopback port, mirroring the
   Google lane convention (ium_core::google::REDIRECT_PORT = 8737):
     ium_core::dropbox::REDIRECT_PORT = 8738
     redirect URI = http://localhost:8738
Next: design note on this issue (contract mapping + token-loss UX), RED at
HEAD, then live wire-facts rehearsal with a console-issued short-lived token.
"""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/{path}",
        data=json.dumps(payload).encode() if payload else None,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


BODY = """## Credential-block record correction — block FULLY LIFTED

**Correction 1 — scope trim is impossible (Dropbox-enforced, accepted).**
The earlier note listing "trim `account_info.read` / `files.metadata.read`" as
an owner console action is wrong: Dropbox enforces `account_info.read` on
every scoped app and auto-includes `files.metadata.read` whenever any
`files.content.*` scope is granted. Owner verified in the console that neither
checkbox can be cleared. This is provider-enforced, not a misconfiguration.

**Final scope set (no further action, GO verdict unchanged):**

| Scope | Status |
|---|---|
| `account_info.read` | forced by Dropbox on scoped apps |
| `files.metadata.read` | auto-included with `files.content.*` |
| `files.content.read` | explicit |
| `files.content.write` | explicit |

Least-privilege note: the app is App-Folder scoped; `account_info.read` on an
App-Folder app returns only the app-folder account info, not full-account
profile data.

**Correction 2 — Allow public clients is ON (owner confirmed).** All console
prerequisites are complete. **Credential block: LIFTED.** The lane proceeds to
design note → RED → live rehearsal → implementation.

**Correction 3 — redirect URI plan (no unregistered guesses, #105 discipline).**
The official OAuth guide: *"You'll need to register the exact redirect URI(s)
your app may use in the Developer Console ... Dropbox checks that the
specified redirect_uri parameter in your Authorization URL matches one of the
registered values."* Exact match **includes the port** — RFC 8252 §7.3-style
variable-port loopback is not available here. The engine therefore binds one
fixed loopback port, mirroring the existing Google lane convention
(`ium_core::google::REDIRECT_PORT = 8737`):

- `ium_core::dropbox::REDIRECT_PORT = 8738`
- registered + requested redirect URI: `http://localhost:8738`

Owner action: paste `http://localhost:8738` under **Settings → OAuth 2 →
Redirect URIs** (a field that has stayed empty until now — nothing else
registered before this verified value).

Next on the lane: design note (contract mapping `start → append_v2 → finish →
content_hash verify` + token-loss UX), RED at HEAD, live rehearsal with a
console-issued short-lived access token (masked in logs).
"""

r = api("POST", "issues/109/comments", {"body": BODY})
print("comment id:", r["id"])
