#!/usr/bin/env python3
"""#110 diagnostic: why does Entra return AADSTS70008 on a fresh code?

Offline checks (no network):
1. callback URL bytes are clean (no newline/space/CR inside the code param)
2. code charset is strict base64url + dots
3. PKCE pairing in .secrets/onedrive_pkce_state.json: sha256(verifier)==challenge
4. decode the authorization code's binary segments and scan for FILETIME /
   epoch-like timestamps to learn the embedded issue/expiry time
"""
import base64
import datetime
import hashlib
import json
import urllib.parse
from pathlib import Path

SECRETS = Path("/home/z/my-project/.secrets")

# --- 1/2: callback hygiene ---
raw = (SECRETS / "onedrive_callback_url.txt").read_bytes()
print(f"callback bytes: {len(raw)}")
print(f"contains CR: {b'\r' in raw}, LF inside query: {raw.count(b'\n')}")
text = raw.decode().strip()
u = urllib.parse.urlparse(text)
qs = urllib.parse.parse_qs(u.query, keep_blank_values=True)
code = qs["code"][0]
print(f"code len: {len(code)}")
ok_charset = all(c in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_." for c in code)
print(f"code charset strict base64url+dots: {ok_charset}")
print(f"code segments (split on '.'): {[len(s) for s in code.split('.')]}")

# --- 3: PKCE pairing ---
st = json.loads((SECRETS / "onedrive_pkce_state.json").read_text())
v = st["verifier"]
calc = base64.urlsafe_b64encode(hashlib.sha256(v.encode()).digest()).rstrip(b"=").decode()
print(f"challenge in file : {st['challenge']}")
print(f"sha256(verifier)  : {calc}")
print(f"PKCE pairing      : {'MATCH' if calc == st['challenge'] else 'MISMATCH!!!'}")
print(f"state in file     : {st['state']}")
print(f"state in callback : {qs['state'][0]}")

# --- 4: decode code binary ---
def b64d(s: str) -> bytes:
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))

parts = code.split(".")
print(f"segments: {len(parts)}")
for i, p in enumerate(parts[:3]):
    try:
        d = b64d(p)
        print(f"--- seg{i} ({len(d)} bytes) head hex: {d[:48].hex()}")
        print(f"    head ascii-ish: {''.join(chr(c) if 32 <= c < 127 else '.' for c in d[:48])}")
    except Exception as e:
        print(f"seg{i} decode fail: {e}")

blob = b64d(parts[2]) if len(parts) > 2 else b""
print(f"--- main blob: {len(blob)} bytes")

# scan for 8-byte big-endian values that decode to sane dates as
# (a) unix-epoch-seconds, (b) windows FILETIME, over the whole blob
now = int(datetime.datetime.now(datetime.timezone.utc).timestamp())
hits = []
for off in range(0, len(blob) - 8):
    val = int.from_bytes(blob[off:off + 8], "big")
    if 1_700_000_000 < val < 1_900_000_000:  # unix seconds 2023-2030
        hits.append((off, val, "unix"))
    elif 133_000_000_000_000_000 < val < 134_500_000_000_000_000:  # FILETIME ~2023-2030
        hits.append((off, val, "filetime"))
    elif 1_700_000_000 < val // 10_000_000 - 11_644_473_600 < 1_900_000_000:
        hits.append((off, val, "filetime~"))
seen = set()
for off, val, kind in hits[:20]:
    if kind == "unix":
        dt = datetime.datetime.fromtimestamp(val, datetime.timezone.utc)
    else:
        dt = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=val // 10)
    key = (off // 8, dt.strftime("%Y-%m-%d %H:%M:%S"))
    if key in seen:
        continue
    seen.add(key)
    print(f"  blob[+{off:4d}] {kind:9s} -> {dt.isoformat()}")
print(f"(now = {datetime.datetime.fromtimestamp(now, datetime.timezone.utc).isoformat()})")
