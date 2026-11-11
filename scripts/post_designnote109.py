#!/usr/bin/env python3
"""Post the #109 design note (adapter contract mapping + token-loss UX)."""
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


BODY = """## Design note — native Dropbox adapter (`adapter_dropbox`)

Design-note-first per the lane. Everything below is grounded in the #108
verdict sources (official docs only) + today's verified redirect rule. No code
in this note.

### 1. Identity & auth

- **Public client, PKCE S256, no secret.** `client_id = dsvpauxp6w4r2ac`
  compiled as a public constant (public identifier, not a credential — the
  app's security is the redirect exact-match + PKCE, not client secrecy).
- **Redirect:** fixed loopback port, mirroring the Google lane
  (`ium_core::google::REDIRECT_PORT = 8737`):
  `ium_core::dropbox::REDIRECT_PORT: u16 = 8738`, URI
  `http://localhost:8738`, registered exactly (Dropbox checks the requested
  `redirect_uri` against registered values; port is part of the exact match).
- **Token path:** code flow with `token_access_type=offline` → refresh token
  into the #81 SecretStore (0600, keyring-backed) — never journal, never log.
  Access-token TTL is read from the wire (`expires_in` on the token response),
  never hardcoded. Refresh fires at a safety margin before expiry and
  write-through on use (rotation-on-use is respected if Dropbox rotates).
- **Scope set (final, provider-enforced):** `account_info.read`,
  `files.content.read`, `files.content.write`, `files.metadata.read`.
- **State guard:** `state` nonce on the authorize URL, checked on the loopback
  callback (CSRF), same shape as the Google lane.

### 2. Adapter contract mapping (ADR-002 — journal is the only truth)

| `TransferAdapter` method | Dropbox wire | Notes |
|---|---|---|
| `probe` | `POST /2/users/get_current_account` | `account_info.read` scope exists for exactly this; proves credential + reachability. Report honest limits (≤150 MiB per request, session 7 days, ~2 TB `too_large` cap). |
| `upload_chunk` (first) | `POST /2/files/upload_session/start` | `session_id` is journaled in the probe event **before** use (transitions journaled before they execute). |
| `upload_chunk` | `POST /2/files/upload_session/append_v2` | cursor `{session_id, offset}`; offset from the journal, never from caller args. |
| offset disagreement | `incorrect_offset` error | carries `correct_offset` — the #10 409-realign pattern: **server wins**, journal records the realigned offset, next append continues there. |
| `finalize` | `POST /2/files/upload_session/finish` | `CommitInfo { path, mode: overwrite, autorename: false }`; response `FileMetadata`. `async: false` first; the async path (`/upload_session/finish_batch/...`) only if sync proves insufficient — honest adapter limit surfaced if we skip async in v1. |
| `verify` | local `content_hash` vs `FileMetadata.content_hash` | **verify-before-declare (ADR-004):** compute the documented algorithm locally (per-4-MiB-block SHA-256, concatenate digests, SHA-256 again, hex) and compare. Mismatch ⇒ NOT Done. If `finish` response lacks the hash (async path), fetch via `POST /2/files/get_metadata`. |
| `reattach` | `append_v2` probe at journaled offset | session ≤7 days old ⇒ server accepts (empty payload, `close=false`) or corrects via `incorrect_offset`; journal replays from the corrected offset. Session expired ⇒ honest re-attach failure → token/session UX below. |
| `uniform_block` | `Some(4 MiB)` | #56 prescribed-geometry channel: keeps chunk boundaries on content-hash block boundaries — local hashing streams block-aligned without cross-chunk carry. |
| `capabilities` | upload-only badge | no `probe_link`, no share URLs (out of scope). |
| `probe_fleet` / `notify_link_bps` | default impls | single-endpoint provider. |

### 3. App-folder path rule

All commits go under the app-folder root (`/Apps/IUM Desktop/...` relative to
the app root — `CommitInfo.path` is app-root-relative for App-Folder apps).
Path is built from the journal's destination key; no user-chosen absolute
paths (scope-minimization honesty).

### 4. Token-loss UX contract (the missing acceptance item)

- **Refresh token absent at start** (never granted, keyring wiped, restore on
  a new machine): job enters `Failed` with the honest sentence: reconnection
  required — the shell offers the browser re-auth (#82 connect surface).
  No silent fallback to the bridge.
- **Refresh revoked/expired mid-job** (revocation, password change, 401 after
  margin-refresh retries): the access-token refresh path retries on the
  retry budget; exhaustion ⇒ `Failed`. Completed chunks are NOT lost — the
  journal holds the Dropbox `session_id` (valid ≤7 days), so after reconnect
  the same job `reattach`es and continues from the journaled offset.
- **Session expired (>7 days) after re-auth:** `reattach` reports the honest
  failure; the job restarts from chunk 0 on the same journal (new session id
  journaled). Restart is recorded, never disguised as resume.
- **No credential ever crosses a log line:** masked to `…<redacted>` in any
  rehearsal or adapter trace.

### 5. Migration & bridge coexistence

Native `dropbox` adapter registers in the adapter registry under its own name;
the #82 bridge path stays until the native lane is verified live, then the
OAuth backend allowlist gains `dropbox`-native and the bridge entry for
dropbox is removed in the SAME commit (no dual paths shipped). Shell exposure
rides the existing connect surface.

### 6. RED plan (next step after this note)

- `ium-core/tests/dropbox_contract.rs`: compile-fail at HEAD (module absent —
  the `verify_is_required.rs` pattern) asserting `ium_core::dropbox::`
  exists with `REDIRECT_PORT = 8738`, the adapter registers as `dropbox`,
  and content-hash vectors (cross-checked against an independent SHA-256
  implementation, generated with Python `hashlib`) match the documented
  algorithm on: empty input, sub-block (<4 MiB), exactly-one-block, and a
  multi-block input.
- RED output pasted here before any implementation lands.

### 7. Live rehearsal (before the adapter merges)

Console-issued short-lived access token (owner), masked in logs; script drives
`start → append_v2 ×2 → finish` on a small file into the App Folder, reads
back `content_hash`, compares to the local computation, and exercises one
deliberate `incorrect_offset` realign. Results = wire-facts ledger rows on
this issue.
"""

r = api("POST", "issues/109/comments", {"body": BODY})
print("design note comment id:", r["id"])
