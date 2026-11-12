#!/usr/bin/env python3
"""Post the #110 design note (Entra unblock facts + Graph contract mapping + token UX)."""
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


BODY = """## BLOCK LIFTED — Entra registration wire facts (owner, 2026-11-12)

- **App:** IUM Desktop — **client_id `c3a906be-d85b-4409-b3cd-bc152ae58550`**
  (public identifier, not a credential; compiled as a public constant, the
  #109 pattern). **No client credentials** — public client, secret-less by design.
- **Account types:** all Microsoft account users (any Entra tenant + personal)
  → authority `https://login.microsoftonline.com/common`.
- **Redirect:** registered bare `http://localhost`, public client/native type —
  the Entra loopback rule (RFC 8252): the port is matched FLEXIBLY for native
  clients, unlike Dropbox's exact-match. The engine still pins a fixed port for
  deterministic UX: `ium_core::onedrive::REDIRECT_PORT: u16 = 8739`
  (google 8737 → dropbox 8738 → onedrive 8739), runtime URI
  `http://localhost:8739`.
- **Client-credential surface on the Overview ("Add a certificate or secret")
  stays EMPTY for this lane's lifetime** — if a secret ever appears there, the
  lane drifted.

## Design note — native OneDrive adapter (`adapter_onedrive`)

Design-note-first per the lane. Grounded in the #108 verdict sources (official
docs only: Graph createUploadSession, the hashes doc, the Entra refresh/access
token pages). No code in this note.

### 1. Identity & auth

- **Public client, PKCE S256, no secret.** Authority `/common` (the
  registration chose all-Microsoft-account-users).
- **Scopes (v2 endpoint): `offline_access Files.ReadWrite User.Read`.**
  `offline_access` is REQUIRED on Entra — without it no refresh token is
  issued at all (the Dropbox analogue of the scope-trim trap; pinned before
  anyone "simplifies" it).
- **Token path:** code flow → tokens into the #81 SecretStore (0600,
  keyring-backed) — never journal, never log.
- **Token lifecycle (the lane's hardest fact):** Entra access tokens are
  **60–90 min, randomized TTL** — a long job WILL cross an expiry mid-flight.
  The client reads `expires_in` from the wire (never hardcoded), refreshes at
  a safety margin, and Entra public-client refresh tokens **rotate on use** —
  every refresh response's new refresh token is written through to the
  SecretStore IMMEDIATELY (rotation-on-use write-through; keeping the old one
  around is the lost-refresh-token bug). A 401 forces one refresh+retry
  before honest classification.

### 2. Adapter contract mapping (ADR-002 — journal is the only truth)

| `TransferAdapter` method | Graph wire | Notes |
|---|---|---|
| `probe` | `GET /v1.0/me/drive` | proves token + reachability + which drive; drive id journaled. |
| `upload_chunk` (first) | `POST /v1.0/me/drive/root:/{path}:/createUploadSession` | `{item: {"@microsoft.graph.conflictBehavior": "replace"}}` (overwrite, the #109 `CommitInfo overwrite` analogue); **`uploadUrl` journaled before use**. |
| `upload_chunk` | `PUT {uploadUrl}` + `Content-Range` | EVERY fragment except the last MUST be a multiple of **320 KiB**; last fragment any size, ≤ 60 MiB. Planned chunks 5 MiB (16 × 320 KiB, inside the official 5–10 MiB band). |
| offset disagreement | HTTP **416** + `nextExpectedRanges` | the #10 realign pattern: **server wins** — journal records the server-confirmed byte position, next PUT continues there. (Rehearsed live below.) |
| `finalize` | the LAST fragment's PUT | Graph has NO separate finish call — the final PUT returns 201/200 + the driveItem. `finalize()` therefore consumes the completion receipt journaled by the last chunk. |
| `verify` | local quickXorHash vs `file.hashes.quickXorHash` | **verify-before-declare (ADR-004):** pure module implements Microsoft's published quickXorHash (base64, 20 bytes); compare against metadata (`GET /v1.0/me/drive/root:/{path}:?$select=id,file` fetches hashes if the completion receipt lacked them). Mismatch ⇒ NOT Done. `sha256Hash` is explicitly not guaranteed — never used. |
| `reattach` | `GET {uploadUrl}` → `nextExpectedRanges` | the uploadUrl IS the session handle (journaled); GET returns the server-confirmed ranges, journal replays from the first missing byte. Session gone (expired/cleaned) ⇒ honest failure → token/session UX below. |
| `uniform_block` | `Some(5 MiB)` | planned chunk size; the hard constraint (320 KiB modulus) is asserted in the contract test: `uniform_block() % 327_680 == 0`. |
| `capabilities` | `RESUMABLE` only | no DYNAMIC_CHUNKS (prescribed geometry), no TAIL_RACE (the final PUT is the atomic commit — no data/commit race window), upload-only badge (no share URLs). |
| `probe_fleet` / `notify_link_bps` | default impls | single-endpoint provider. |

### 3. Path rule

`Files.ReadWrite` is full-drive scope (no App-Folder analogue): the commit
path is `{journal destination}` under the user's drive root — scope is wider
than Dropbox's app folder, so the shell surfaces the destination folder to the
user at connect time (honest scope display, the #103 guided-keys pattern).

### 4. Token-loss UX contract

- **Refresh token absent at start** (never granted, keyring wiped, new
  machine): job enters `Failed` with the honest reconnection sentence — the
  shell offers the browser re-auth (#82 connect surface). No silent bridge
  fallback.
- **401 mid-job:** one refresh+retry (rotation written through first); retry
  budget exhaustion ⇒ `Failed`. Completed fragments are NOT lost — the
  journal holds `uploadUrl`; after re-auth the same job `reattach`es via
  `nextExpectedRanges` and continues from the journaled byte.
- **Session expired after re-auth:** `reattach` reports the honest failure;
  job restarts from chunk 0 on the same journal (new uploadUrl journaled).
  Restart is recorded, never disguised as resume. (Graph does not publish a
  session TTL on the wire — the rehearsal ledger records what we observe, the
  adapter never assumes one.)
- **No credential ever crosses a log line:** masked to `…<redacted>` in any
  rehearsal or adapter trace.

### 5. Migration & bridge coexistence

Native `onedrive` adapter registers under its own name; the bridge entry for
OneDrive leaves `OAUTH_BACKENDS`/`is_oauth_backend` in the SAME commit that
lands the verified native lane (no dual paths shipped — the #109 shape).

### 6. RED plan (next step after this note)

- `ium-core/tests/onedrive_contract.rs`: compile-fail at HEAD (module absent —
  the `verify_is_required.rs` pattern) asserting `ium_core::onedrive::` exists
  with `REDIRECT_PORT = 8739`, `redirect_uri() == "http://localhost:8739"`,
  the capabilities bits, `uniform_block() % 327_680 == 0`, and quickXorHash
  vectors (pure-module discipline, the #6 sigv4 pattern: implementation
  cross-checked against an INDEPENDENT implementation written from the
  official Microsoft algorithm page; vectors pinned in the test).
- RED output pasted here before any implementation lands.

### 7. Live rehearsal (before the adapter merges)

PKCE via the owner (no console token exists on the Entra side): authorize URL
with `offline_access Files.ReadWrite User.Read` → owner consents → the
loopback redirect URL is pasted back → code+verifier exchange (masked logs).
Script then drives `me/drive → createUploadSession → 2 fragments + tail →
one DELIBERATE wrong Content-Range (416 + nextExpectedRanges realign) →
correct final PUT → metadata quickXorHash vs locally computed`. Results =
wire-facts ledger rows on this issue.
"""

r = api("POST", "issues/110/comments", {"body": BODY})
print("design note comment id:", r["id"])
