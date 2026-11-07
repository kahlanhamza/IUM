#!/usr/bin/env python3
"""GREEN evidence + CI verdict for #84 (Google Drive) — commit f7285e8, gate.sh GREEN, CI success. Closes #59 too."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**GREEN** — landed at f7285e8 (`feat(app,shell): the Google Drive lane — one-click Connect, the real Google sign-in`), `Closes #84, closes #59` in the body — **#59 closed with this push** (the shelve is lifted, the lane shipped).

**Gates, all local on the committed tree:**
- `cargo test -p ium-core` — **290 green** (+2 google_drive: the env-contract builder `RCLONE_CONFIG_IUM_DRIVE_{TYPE,TOKEN}` verbatim + remote `IUM_DRIVE:` + the required-token refusals); fmt clean; clippy `-D warnings`.
- vitest **97/97** (the flipped allowlist pins + the hidden-token-row contracts + the settings hidden-carry rules) · tsc strict · vite build 201.92 KB.
- `bash scripts/gate.sh` — **GREEN on the committed tree f7285e8**.

**CI:** run on f7285e8 — **completed / success** ([run 42415892021](https://github.com/kahlanhamza/IUM/actions/runs/42415892021)).

**What a user sees (the owner's bar, word for word):** Settings → **Google Drive** row → Connect → the sheet has NO fields — one button, **Connect Google Drive** → the browser opens Google's real sign-in (2FA and consent included) → on success the drawer **closes itself** and the lane is live. The token rides the OS keychain; its row is a saved-state chip, never an input; **Test** proves the saved sign-in with a read-only probe. Uploads ride the rclone drive backend (chunked; resumability honest-by-capability) under remote `IUM_DRIVE:` — the user's own `rclone.conf` namespace is never touched.

**Honest scope notes (also on the issue design note):** rclone's default Drive scope (the user consents on Google's own screen); tightening to `drive.file` is a follow-up. The four #82 platforms keep their picker; Drive also still rides the generic bridge's merged env lines (one credential, two honest seats).

Next in the owner's ruling: #85 (YouTube — native resumable adapter + Google OAuth loopback sign-in).
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/84/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted GREEN evidence: {out['html_url']}")
