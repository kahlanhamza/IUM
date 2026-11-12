#!/usr/bin/env python3
"""#110 rehearsal: redeem PKCE authorization code -> token pair (secrets stay local, 0600).

- Validates state against .secrets/onedrive_pkce_state.json
- Redeems code with code_verifier at the v2.0 token endpoint (public client, no secret)
- Persists access+refresh tokens to .secrets/onedrive_rehearsal_token.json (0600)
- Prints ONLY masked facts (lengths + 6-char head); never prints tokens
- Probes GET /v1.0/me/drive for wire facts (driveType, driveId, quota)
"""
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

SECRETS = Path("/home/z/my-project/.secrets")
TOKEN_URL = "https://login.microsoftonline.com/common/oauth2/v2.0/token"
DRIVE_URL = "https://graph.microsoft.com/v1.0/me/drive"

client_id = (SECRETS / "onedrive_client_id").read_text().strip()
st = json.loads((SECRETS / "onedrive_pkce_state.json").read_text())

cb_file = SECRETS / "onedrive_callback_url.txt"
cb_file.chmod(0o600)
qs = urllib.parse.parse_qs(urllib.parse.urlparse(cb_file.read_text().strip()).query)
code = qs["code"][0]
returned_state = qs["state"][0]

if returned_state != st["state"]:
    print("STATE MISMATCH -> refusing to redeem (possible CSRF). Aborting.")
    sys.exit(1)
print("state: MATCH")

data = urllib.parse.urlencode({
    "grant_type": "authorization_code",
    "code": code,
    "redirect_uri": st["redirect_uri"],
    "client_id": client_id,
    "code_verifier": st["verifier"],
}).encode()

req = urllib.request.Request(TOKEN_URL, data=data, headers={
    "Content-Type": "application/x-www-form-urlencoded",
})
try:
    resp = json.loads(urllib.request.urlopen(req, timeout=30).read())
except urllib.error.HTTPError as e:
    body = e.read().decode(errors="replace")
    print(f"exchange FAILED: HTTP {e.code}")
    print(body[:600])  # error codes/descriptions only, no secrets in these
    sys.exit(1)

tok = {
    "access_token": resp["access_token"],
    "refresh_token": resp.get("refresh_token"),
    "expires_in": resp["expires_in"],
    "token_type": resp["token_type"],
    "scope": resp.get("scope", ""),
    "obtained_at": int(time.time()),
}
out = SECRETS / "onedrive_rehearsal_token.json"
out.write_text(json.dumps(tok) + "\n")
out.chmod(0o600)


def mask(s: str) -> str:
    return f"(len={len(s)}, head={s[:6]}...)" if s else "(none)"


print(f"token_type    : {tok['token_type']}")
print(f"expires_in    : {tok['expires_in']}s")
print(f"scope granted : {tok['scope']}")
print(f"access_token  {mask(tok['access_token'])}")
print(f"refresh_token {mask(tok['refresh_token'])}")
if not tok["refresh_token"]:
    print("FAIL: no refresh token (offline_access not granted)")
    sys.exit(1)
print("refresh token present: True (write-through to SecretStore is the #81 pattern)")

req2 = urllib.request.Request(DRIVE_URL, headers={
    "Authorization": f"Bearer {tok['access_token']}",
})
try:
    drive = json.loads(urllib.request.urlopen(req2, timeout=30).read())
except urllib.error.HTTPError as e:
    print(f"probe /me/drive FAILED: HTTP {e.code} {e.read().decode(errors='replace')[:300]}")
    sys.exit(1)

print("--- me/drive wire facts ---")
print(f"driveType : {drive.get('driveType')}")
print(f"driveId   : {drive.get('id')}")
q = drive.get("quota", {})
print(f"quota     : total={q.get('total')} used={q.get('used')} remaining={q.get('remaining')} state={q.get('state')}")
print("(displayName / userPrincipalName intentionally not printed)")
