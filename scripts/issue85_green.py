#!/usr/bin/env python3
"""GREEN evidence + CI verdict for #85 (YouTube) — commit 32137f7, gate.sh GREEN, CI success. Closes #60 too."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**GREEN** — landed at 32137f7 (`feat(app,core): the YouTube lane — native resumable upload + the loopback Google sign-in`), `Closes #85, closes #60` in the body — **#60 closed with this push**. The owner's ruling is now fully shipped: **WebDAV, Google Drive AND YouTube — upload by connecting the account.**

**Gates, all local on the committed tree:**
- `cargo test -p ium-core` — **313 green** (+25: 13 wire contracts incl. the RFC 7636 vector, the consent-URL shape, the CSRF refusal, the resumable Content-Range/308 language, the 100-char title cap, the honest capability declaration (8 MiB chunks, 256 KiB grain, max_parallel 1), the loud credential refusals — + 5 google.rs unit tests + 7 adapter unit tests).
- `cargo fmt --all -- --check` clean · `cargo clippy -p ium-core --all-targets -- -D warnings` clean · `cargo doc -D warnings` clean (the doc lane caught 1 broken intra-doc link pre-push — fixed).
- vitest **97/97** · tsc strict · vite build 203.02 KB (+1.1 KB the loopback sign-in surface).
- `bash scripts/gate.sh` — **GREEN on the committed tree 32137f7**.

**CI:** run on 32137f7 — **completed / success** ([run 42420977711](https://github.com/kahlanhamza/IUM/actions/runs/42420977711)) — the shell's Rust compile proof on all 3 OS, including the new per-OS browser-opener branches.

**What a user sees:** Settings → **YouTube** row → Connect → the sheet has two one-time fields (client id, client secret — the honest BYO setup until IUM ships a registered, Google-verified client; the sheet says so in one sentence) → **Connect YouTube** opens Google's real consent screen in the browser → the redirect returns to `localhost:8737` (PKCE + anti-CSRF state) → the token lands in the OS keychain → the drawer **closes itself** and the lane is live. Uploads ride the native resumable adapter: 8 MiB sequential PUTs, the `308 + Range` resume discipline, verify = `videos.list` status — private by default.

**The three lanes, on the record:** #83 WebDAV (50e3765) · #84 Google Drive (f7285e8, closes #59) · #85 YouTube (32137f7, closes #60). Every one full loop: issue → design note → RED at HEAD → implementation → gates → CI verdict → STATUS.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/85/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted GREEN evidence: {out['html_url']}")
