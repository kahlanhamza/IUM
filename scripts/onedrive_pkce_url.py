#!/usr/bin/env python3
"""Generate the #110 rehearsal PKCE material + authorize URL (secrets stay local, 0600)."""
import base64
import hashlib
import json
import secrets
import urllib.parse
from pathlib import Path

CLIENT_ID = Path("/home/z/my-project/.secrets/onedrive_client_id").read_text().strip()
REDIRECT_URI = "http://localhost:8739"
SCOPE = "offline_access Files.ReadWrite User.Read"
AUTHORITY = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"

verifier = base64.urlsafe_b64encode(secrets.token_bytes(48)).rstrip(b"=").decode()
challenge = base64.urlsafe_b64encode(
    hashlib.sha256(verifier.encode()).digest()
).rstrip(b"=").decode()
state = secrets.token_urlsafe(24)

state_file = Path("/home/z/my-project/.secrets/onedrive_pkce_state.json")
state_file.write_text(json.dumps({
    "verifier": verifier,
    "challenge": challenge,
    "state": state,
    "redirect_uri": REDIRECT_URI,
    "scope": SCOPE,
}) + "\n")
state_file.chmod(0o600)

params = urllib.parse.urlencode({
    "client_id": CLIENT_ID,
    "response_type": "code",
    "redirect_uri": REDIRECT_URI,
    "response_mode": "query",
    "scope": SCOPE,
    "code_challenge": challenge,
    "code_challenge_method": "S256",
    "state": state,
})
print(AUTHORITY + "?" + params)
