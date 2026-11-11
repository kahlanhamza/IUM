#!/usr/bin/env python3
"""Post the #109 GREEN receipt comment + update the STATUS row with the CI id."""
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


BODY = """## GREEN — the lane shipped (`273b073`, CI run 43116648434)

- **Local gate on the COMMITTED tree** (non-bypassable): fmt + clippy `-D
  warnings` + full core suite (52 targets green, incl. the new
  `dropbox_contract` 3/3 + dropbox unit 9/9) + doc lint + tsc/vite + vitest
  **191/191** — shell rust lane loud-skip (WebKitGTK absent in the sandbox).
- **CI run 43116648434 = SUCCESS, 8/8**, on the same SHA — `ium-app`
  compiled natively on ubuntu / macOS / windows (the lane the sandbox
  skipped), `core` green on all three OSes, `app-e2e` + `fmt` green.
- Acceptance: design note ✅ (6808543416) · RED at HEAD ✅ (6808729783) ·
  live rehearsal ✅ (6808726696) · adapter behind standard gates ✅ (the
  trait surface did NOT move — no trybuild extension needed, ADR-013 shape)
  · verify refuses Done without the `content_hash` match ✅ (pinned in the
  contract test, proven live at rehearsal step 9) · gates + CI verdict ✅ ·
  shell exposure rides the connect surface with the bridge entry removed in
  the SAME commit ✅.

The lane stands: users sign in with their Dropbox account against IUM's OWN
app identity (PKCE public client, no secret anywhere), uploads land in the
app folder, resume rides the journaled session id, and no job declares Done
unless the server's `content_hash` equals the locally computed one.

Next on the board: **#110 (OneDrive)** — same recipe, blocked on the Entra
app registration (owner console action).
"""

r = api("POST", "issues/109/comments", {"body": BODY})
print("GREEN receipt comment id:", r["id"])

# STATUS row receipt: add the CI id
status = Path("/home/z/my-project/STATUS.md")
s = status.read_text()
old = "rehearsal script `scripts/rehearsal109b.py` (raw ledger kept LOCAL ONLY — it contains the owner's account data, never committed); gate GREEN on the committed tree | this commit |"
new = "rehearsal script `scripts/rehearsal109b.py` (raw ledger kept LOCAL ONLY — it contains the owner's account data, never committed); gate GREEN on the committed tree; CI 43116648434 SUCCESS 8/8 on the same SHA (ium-app native on all 3 OS — the skipped lane owned) | 273b073 |"
assert old in s, "STATUS receipt anchor not found"
status.write_text(s.replace(old, new, 1))
print("STATUS receipt updated")
