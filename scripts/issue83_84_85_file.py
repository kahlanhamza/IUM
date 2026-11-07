#!/usr/bin/env python3
"""File the three connect-accounts lanes the owner just authorized:
#83 WebDAV, #84 Google Drive (implements #59), #85 YouTube (implements #60).
Body text lives here (script file, not heredoc — the backtick lesson)."""
import json
import urllib.request

TOK = open(".secrets/gh_token").read().strip()
REPO = "kahlanhamza/IUM"

ISSUES = [
    {
        "title": "WebDAV adapter: the rclone webdav lane (URL + username + password, Test connection, hot apply)",
        "body": """## The lane
Next queued adapter lane: WebDAV — Nextcloud, ownCloud, generic DAV servers
(Apache/nginx-dav, Synology, Hoverfly…). It rides the proven rclone bridge
(#63) exactly like the MEGA lane (#61 D1), with one honest caveat the owner's
own framing already pinned: WebDAV is a PROTOCOL, not a service — there is no
OAuth to point at, so this lane keeps a focused credential sheet (the #82
surface: one row, one action, a sheet that closes itself on a green probe).

## Design (mirror mega end-to-end)
1. **Core** — `adapter_rclone.rs`:
   - `WEBDAV_LANE: &str = "ium_webdav"` (registry key, `'static`).
   - `webdav_from_env()` → `RcloneBridgeConfig`: reads `IUM_WEBDAV_URL` +
     `IUM_WEBDAV_USER` + `IUM_WEBDAV_PASS` (required — `AuthRequired` when
     missing), `IUM_WEBDAV_VENDOR` (optional: nextcloud|owncloud|sharepoint),
     `IUM_WEBDAV_BUCKET` (default `ium`), `IUM_WEBDAV_RCLONE_BIN` (default
     `rclone`). Remote = the DAV root URL; env lines
     `RCLONE_CONFIG_IUM_WEBDAV_{TYPE,URL,VENDOR,USER,PASS}` — the pass rides
     **rclone-obscured** (same subprocess practice as MEGA, never logged).
   - `normalize_vendor()` pure helper (trim/lowercase; empty → generic DAV).
2. **Vault** — schema entry `webdav`: url (required), username (required),
   password (secret, required), vendor (optional), target folder (optional);
   `apply_env` maps the five `IUM_WEBDAV_*` names (env-wins precedence
   unchanged).
3. **Shell** — `build_adapter("webdav")` arm (block_on webdav_from_env +
   start), registration list, and the `probe_credentials` arm: builds the
   config from DRAFTS, obscures the pass, runs the read-only bridge probe —
   no upload session can ever be orphaned by a Test button.
4. **UI** — schema-driven (the sheet renders itself); `friendlyAdapter`
   gains `webdav: "WebDAV"`.

## RED (pinned first, failing at HEAD)
- vitest: schema rows expose the webdav lane — url/username/password
  required, password secret (fails at HEAD: no such lane).
- cargo test: `webdav_from_env` with no envs → `AdapterError::AuthRequired`
  (compile-fail at HEAD: no such constructor).

## Gates
fmt + clippy + cargo test + tsc/vitest/vite build on the committed tree;
CI on 3 OS.
""",
    },
    {
        "title": "Google Drive lane: one-click Connect — the real Google sign-in in your browser (implements #59)",
        "body": """## The ruling, lifted
#59 was owner-shelved ("Google types absent by owner ruling"). TODAY the
owner lifted it: *"Go ahead with the WebDAV, also with Google drive and
YouTube, if it is possible for user to upload just by connecting his
accounts."* The bar stays the #82 bar: click Connect → the browser opens
Google's real sign-in (2FA and consent included) → the app finishes by
itself and the lane is live. No fields.

## Design
1. **Allowlist flip, both readers** — the injection contract's two ends:
   `auth.rs is_oauth_backend` gains `"drive"`; `connect.ts OAUTH_BACKENDS`
   gains the Google Drive entry. The tests that today PIN drive OUT
   (`!is_oauth_backend("drive")`) are the RED — they flip WITH the feature.
2. **First-class lane row** — vault schema entry `google_drive` with a
   hidden required `token` row: `FieldSchema`/`FieldRow` gain `hidden`
   (no input renders), and the row's PRESENCE is derived from the saved
   `RCLONE_CONFIG_IUM_DRIVE_TOKEN` line in the bridge lane's `extra_env`
   (saved truth only — drafts are not values; the UI never sees a token
   byte). `connectionState` then reads it like any lane: Not set up →
   Connected.
3. **Sign in** — the sheet for `google_drive` renders a Google-only sign-in
   panel (no picker): the shell spawns `rclone authorize drive` on the
   blocking pool (rclone's own embedded OAuth client — zero config), parses
   the token (#82's pure-std parser), and on success:
   - merges the env lines into the bridge lane's `extra_env` (inside the
     process, #69),
   - stores the token JSON under `google_drive.token` (secret → OS keychain,
     #81),
   - hot-applies the `google_drive` lane — the drawer closes itself (the
     owner's reflex).
4. **Live lane** — core `google_drive_from_env()`:
   `IUM_GOOGLE_DRIVE_TOKEN` (required) → `RcloneBridgeConfig` remote
   `IUM_DRIVE:`, env lines `RCLONE_CONFIG_IUM_DRIVE_{TYPE,TOKEN}`,
   `lane_name = GOOGLE_DRIVE_LANE`. Uploads ride the rclone drive backend —
   chunked by rclone; resumability honest-by-capability.
5. **Test connection** — `probe_credentials("google_drive")` reads the
   SAVED token (a hidden row has no draft), builds the config, runs the
   read-only bridge probe.

## Honest scope notes
- Scope: rclone's default Drive scope (the user consents on Google's own
  screen). Tightening to `drive.file` is a follow-up issue, not this lane.
- Drive is a bucket-less backend: the bridge serves from the root with a
  top-level folder as the bucket (the MEGA topology wire fact).

## RED
- `auth.rs` allowlist test pins `!is_oauth_backend("drive")` → flips.
- `connect.test.ts` pins the four ids → flips to five.
- vitest: google_drive lane present with hidden token row (fails at HEAD).
- cargo test: `google_drive_from_env` no-env → `AuthRequired` (compile-fail
  at HEAD).
""",
    },
    {
        "title": "YouTube lane: native resumable upload adapter + Google OAuth sign-in (implements #60)",
        "body": """## The ruling, lifted
#60 was owner-shelved. TODAY the owner lifted it, same bar: upload by
connecting the account. rclone has NO youtube backend — the browser sign-in
cannot ride `rclone authorize` for this lane. YouTube becomes IUM's first
NATIVE OAuth adapter.

## Design
1. **Core `google.rs`** (Google OAuth, installed-app class):
   - pure: `authorize_url()` (PKCE S256 + state, `youtube.upload` scope),
     `pkce_challenge()` (verifier → S256 base64url), `parse_redirect()`
     (code + state, mismatch-refusing), `TokenSet` JSON parse.
   - reqwest (the TLS dep already in core): `exchange_code()`,
     `refresh_token()` — `oauth2.googleapis.com/token`.
   - The loopback redirect server is SHELL-side: std `TcpListener` on
     `127.0.0.1:8737`, the OS default opener launches the browser (no new
     shell dependency); the landing page is one honest sentence.
2. **Core `adapter_youtube.rs`** — `TransferAdapter` registry key
   `"youtube"`:
   - probe = `channels.list?part=id&mine=true` (read-only credential check).
   - upload = `youtube/v3/videos?uploadType=resumable&part=snippet,status`:
     init POST (title/description/tags/categoryId + privacyStatus) →
     session Location → 8 MiB PUTs with `Content-Range` → on failure the
     `bytes */total` resume probe (308 + `Range`) → resume — the
     download-manager reliability story on YouTube's own protocol.
   - verify-before-declare: `videos.list?part=status` (uploaded/processed) —
     honest-by-capability: YouTube offers no server-side file checksum, so
     verify is resource existence + status, and the lane says so.
   - refresh-on-401 with counted retries; `privacyStatus` default **private**;
     title derived from the filename (sanitized, ≤100 chars).
   - env: `IUM_GOOGLE_CLIENT_ID` + `IUM_GOOGLE_CLIENT_SECRET` +
     `IUM_YOUTUBE_TOKEN` (TokenSet JSON).
3. **Vault** — schema `youtube`: client id (required), client secret
   (secret, required), token (hidden, secret, required — set by sign-in);
   `apply_env` maps the three names.
4. **UI** — the youtube sheet: the two client fields + one
   Sign-in-with-Google button (loopback flow) + the hidden token row.

## Honest BYO-client note (ships in the UI too)
Google has no anonymous OAuth: every flow needs a registered client.
rclone ships its own Drive client, which is why #84 needs zero config —
YouTube gets no such ride. Until IUM ships a registered (and for
`youtube.upload`, Google-verified) client, the two client fields are a
ONE-TIME setup; after that, connect is one click. Unverified projects are
limited to test users (100) — the sheet says so in one sentence.

## RED
- cargo test: PKCE challenge vector (RFC 7636 appendix B), redirect parse
  refusals, init-body builder, content-range builder, 308/Range resume
  parse, chunk math, title sanitizer (compile-fail at HEAD).
- vitest: youtube lane rows + connect-sheet copy (fails at HEAD).
""",
    },
]


def post(path, payload):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": f"Bearer {TOK}",
            "Accept": "application/vnd.github+json",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


for i in ISSUES:
    out = post("issues", i)
    print(f"#{out['number']} OPEN  {out['title'][:80]}")
