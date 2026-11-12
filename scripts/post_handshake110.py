#!/usr/bin/env python3
"""Post the #110 handshake-blocker wire-facts comment (AADSTS70008 forensics)."""
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


BODY = """## Handshake blocker — `AADSTS70008` ×2, second one on a FRESH code; forensics banked; rehearsal stays the close condition

Two redemption attempts today, both `invalid_grant` / `AADSTS70008` ("expired
due to inactivity"):

1. **Code #1** — owner paste delayed well past the ~10-min code TTL: genuine
   expiry, expected, logged.
2. **Code #2** — pasted back within minutes of the authorize round-trip (same
   live Entra session, `session_state 00a98e5a…` in both callbacks): TTL
   expiry IMPOSSIBLE. The error is a mask for something else.

Offline forensics (`scripts/onedrive_diag_70008.py`, zero network):

- callback bytes clean: no CR/LF mangling anywhere in the URL; code = 3
  dot-separated segments, 1383 chars, strict base64url charset;
- **PKCE pairing VERIFIED**: `sha256(verifier) == code_challenge`
  (`veVUKxnB…`) — the material in the state file is provably the pair the
  authorize URL carried;
- **state round-trip VERIFIED** (`nzGGvSrt…`);
- the code's binary blob scanned for embedded unix/FILETIME timestamps —
  none found readable (structure header `EvoStsArtifacts`), so the expiry
  claim cannot be confirmed from the artifact itself.

Conclusion: the paste path and the PKCE material are PROVEN good; the failure
is server-side. Leading suspects, in order:

- **(a) the registration's `Allow public client flows` toggle is still NO** —
  it was never confirmed on the record, and Entra surfaces some grant-level
  refusals as 70008;
- (b) authorize-round invalidation: a second authorize completing for the
  same session/client/redirect retires the earlier code.

**Next attempt — the device-code pivot (no URL paste race at all):**
`POST /common/oauth2/v2.0/devicecode` → owner opens
`microsoft.com/devicelogin` on the machine that is ALREADY signed in, types
the short code, consents once; the engine polls the token endpoint
automatically until `authorization_pending` clears. No redirect URI, no port,
no TTL race. Requires the same toggle (a).

The 30-second checklist on the app registration, for the record:

- Authentication → **Allow public client flows = Yes**
- API permissions: `offline_access` + `Files.ReadWrite` + `User.Read`
  (`offline_access` must be added explicitly — it is NOT a default, and
  without it Entra issues no refresh token at all)

**Lane state:** implementation + contract GREEN locally (full gates: fmt ✓,
clippy -D warnings ✓, `cargo test -p ium-core` 53/53 targets ✓, contract
6/6). The push proceeds with the live rehearsal marked **PENDING** — nothing
is declared verified that isn't. #110 stays OPEN until the rehearsal lands
its wire-facts rows here (probe → createUploadSession → fragments →
deliberate 416 realign → final PUT → metadata `quickXorHash` vs local).
"""

r = api("POST", "issues/110/comments", {"body": BODY})
print("handshake-blocker comment id:", r["id"])
