#!/usr/bin/env python3
"""File #82 — Connect, don't configure (owner's Google-style auth ask)."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """Owner's bar (verbatim, 2026-11-07):

> to connect an account you must click connect and a window should be popped up and user can connect to there account as normal after finishes the window closes and the app is connected like for exemple the google authentication style, they are so easy, and smoth

> I know that Oauth is not meant for every adapter we support, for exemple S3 might be any service so it's hard to point it to a specific service ... but Im just talking about obvious ones

## Wire facts (what #81 already landed)

- Keychain-backed secrets (`SecretStore` seam, the `keyring` crate, honest 0600-file fallback), hot lanes (save = live apply, no restart), and a read-only Test-connection probe before the first upload.
- The gap this issue closes: every lane still renders its full field wall by default, the connect action has no single entry point, and OAuth-capable backends have no browser sign-in path at all.

## Plan (issue-first, RED first)

1. **`connect.ts` (pure, vitest)** — `connectionState(rows)` (connected / partial / empty from required-field presence), the OAuth backend list (pCloud, Dropbox, OneDrive, Box — **Google types stay absent: owner-shelved #59/#60**), `oauthEnvLines` + `mergeExtraEnv` (rclone's own env-config practice: `RCLONE_CONFIG_<REMOTE>_type` / `_token`).
2. **`auth.rs` (pure-std, the rustc `--test` harness)** — `parse_authorize_output`: extracts the token JSON `rclone authorize <backend>` prints after the browser round-trip, validates shape (`type` + `token`), refuses garbage; `env_lines` builder. Zero dependencies so the local harness stays honest.
3. **`authorize_oauth` command (lib.rs)** — spawns `rclone authorize <backend>` with the bridge's bin resolution, captures stdout, parses, returns the env lines. The token rides `extra_env` — already a secret field, so it lands in the OS keychain like every other secret.
4. **The connect sheet (SettingsDialog)** — each lane collapses to one honest row: state chip (Connected / Not set up) + ONE primary action. Connect opens a focused single-lane sheet: only that lane's fields, inline Test connection, one Save-and-connect action; on a successful connect the drawer CLOSES ITSELF (the "the window closes and the app is connected" reflex). The rclone-bridge lane gains the browser-sign-in path with the waiting-state copy while the user signs in.

## Honest-by-capability (the owner's own framing)

| Path | Lanes | Why |
| --- | --- | --- |
| Browser sign-in popup | pCloud, Dropbox, OneDrive, Box (via the rclone bridge) | an obvious single identity to point at |
| Focused credential sheet | S3 (any provider), TeraBox, MEGA, Feishu, WebDAV, TUS/HTTP | generic protocols — there is no single OAuth identity to redirect to; one screen, autofill, Test, done |

Acceptance: vitest RED→GREEN on the pure module; the rustc `--test` harness green on the parser; tsc + vite build green; `scripts/gate.sh` green on the committed tree; the drawer closes itself on a successful connect; shelved Google types appear nowhere in the surface.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({
        "title": "Connect, don't configure: one-click connect sheets + browser sign-in for the obvious OAuth backends",
        "body": BODY,
        "labels": [],
    }).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"filed #{out['number']}: {out['title']}")
print(f"url: {out['html_url']}")
