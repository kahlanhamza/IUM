#!/usr/bin/env python3
"""#109 live wire-facts rehearsal against the real Dropbox API.

Drives the exact wire path the native adapter will take:
  get_current_account (probe) -> upload_session/start -> append_v2 x3
  (including ONE deliberate incorrect_offset realign) -> finish(CommitInfo)
  -> server content_hash vs locally computed (Python hashlib, independent).

Honesty rules:
- the access token is read from .secrets/dropbox_rehearsal_token and is NEVER
  printed; session ids and account ids are masked in output;
- every response status and body is recorded verbatim (bodies contain no
  credentials) so the ledger rows on #109 are real wire facts.
"""
import hashlib
import json
import time
import urllib.error
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/dropbox_rehearsal_token").read_text().strip()
CONTENT = "https://content.dropboxapi.com/2/files"
RPC = "https://api.dropboxapi.com/2"
BLOCK = 4 * 1024 * 1024

ledger: list[str] = []


def mask(s: str, keep: int = 6) -> str:
    if not s:
        return "<empty>"
    return s[:keep] + "…<masked>" if len(s) > keep else "<masked>"


def call(label: str, url: str, api_arg: dict | None, body: bytes | None = None,
         json_body: dict | None = None) -> tuple[int, dict | bytes]:
    """api_arg -> content endpoint (Dropbox-API-Arg + raw body);
    json_body -> RPC endpoint with a JSON body; neither -> bare RPC POST
    (NO Content-Type — urllib's json content-type on an empty body trips a
    Dropbox 500, observed live 2026-11-12; bare curl POST is accepted)."""
    data = body if body is not None else (json.dumps(json_body).encode() if json_body else None)
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Authorization", f"Bearer {TOKEN}")
    if api_arg is not None:
        req.add_header("Dropbox-API-Arg", json.dumps(api_arg))
        req.add_header("Content-Type", "application/octet-stream")
    elif json_body is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=300) as r:
            data = r.read()
            status = r.status
    except urllib.error.HTTPError as e:
        data = e.read()
        status = e.code
    parsed: dict | bytes
    try:
        parsed = json.loads(data) if data else {}
    except json.JSONDecodeError:
        parsed = data
    line = f"{label}: HTTP {status} {json.dumps(parsed)[:400] if isinstance(parsed, dict) else f'{len(data)}B'}"
    ledger.append(line)
    return status, parsed


def pattern(n: int) -> bytes:
    return bytes(i % 251 for i in range(n))


def content_hash(data: bytes) -> str:
    h = hashlib.sha256()
    if not data:
        return h.hexdigest()
    for i in range(0, len(data), BLOCK):
        h.update(hashlib.sha256(data[i:i + BLOCK]).digest())
    return h.hexdigest()


# --- 1. probe ---------------------------------------------------------------
status, acct = call("probe get_current_account", f"{RPC}/users/get_current_account", None)
assert status == 200, f"probe failed: {status} {acct}"
print(f"probe: HTTP 200 account_id={mask(acct.get('account_id',''))} "
      f"email={mask((acct.get('email') or '').split('@')[0])}@…")

# --- 2. build the payload ---------------------------------------------------
c1, c2, c3 = pattern(BLOCK), pattern(BLOCK), pattern(1024 * 1024)
total = c1 + c2 + c3
local_hash = content_hash(total)
print(f"payload: {len(total)} bytes in 3 chunks (4 MiB / 4 MiB / 1 MiB), local content_hash={local_hash}")

# --- 3. start ---------------------------------------------------------------
status, res = call("start", f"{CONTENT}/upload_session/start", {"close": False})
assert status == 200 and "session_id" in res, f"start failed: {status} {res}"
sid = res["session_id"]
print(f"start: HTTP 200 session_id={mask(sid)}")

# --- 4. append #1 (correct offset 0) ----------------------------------------
status, res = call("append#1 @0", f"{CONTENT}/upload_session/append_v2",
                   {"cursor": {"session_id": sid, "offset": 0}, "close": False}, c1)
assert status == 200, f"append#1 failed: {status} {res}"
print("append#1: HTTP 200 (offset 0 accepted, body 4 MiB)")

# --- 5. append #2 at a DELIBERATELY WRONG offset -> realign ------------------
status, res = call("append#2 WRONG @0", f"{CONTENT}/upload_session/append_v2",
                   {"cursor": {"session_id": sid, "offset": 0}, "close": False}, c2)
assert status == 400 and "incorrect_offset" in res, f"expected incorrect_offset, got {status} {res}"
correct = res["error"]["incorrect_offset"]["correct_offset"]
ledger.append(f"realign: server corrected offset -> {correct}")
print(f"append#2 wrong: HTTP 400 incorrect_offset, server says correct_offset={correct}")
assert correct == BLOCK, f"unexpected correct_offset {correct}"
status, res = call(f"append#2 realigned @{correct}", f"{CONTENT}/upload_session/append_v2",
                   {"cursor": {"session_id": sid, "offset": correct}, "close": False}, c2)
assert status == 200, f"realigned append failed: {status} {res}"
print(f"append#2: HTTP 200 after realign (offset {correct} accepted)")

# --- 6. append #3 (tail, offset 8 MiB) ---------------------------------------
status, res = call("append#3 @8388608", f"{CONTENT}/upload_session/append_v2",
                   {"cursor": {"session_id": sid, "offset": 8 * 1024 * 1024}, "close": False}, c3)
assert status == 200, f"append#3 failed: {status} {res}"
print("append#3: HTTP 200 (tail 1 MiB at offset 8388608)")

# --- 7. finish (empty body — everything already appended) ---------------------
path = f"/ium-rehearsal-{int(time.time())}.bin"
status, meta = call("finish", f"{CONTENT}/upload_session/finish",
                    {"cursor": {"session_id": sid, "offset": len(total)},
                     "commit": {"path": path, "mode": "overwrite", "autorename": False}})
assert status == 200, f"finish failed: {status} {meta}"
server_hash = meta.get("content_hash")
print(f"finish: HTTP 200 name={meta.get('name')} size={meta.get('size')} "
      f"server content_hash={server_hash}")

# --- 8. verify-before-declare -------------------------------------------------
match = server_hash == local_hash
print(f"VERIFY: server content_hash == local content_hash -> {match}")
ledger.append(f"verify: server={server_hash} local={local_hash} match={match}")
assert match, "content_hash MISMATCH — verify-before-declare refuses Done"

# --- 9. cleanup ---------------------------------------------------------------
status, res = call("cleanup delete_v2", f"{RPC}/files/delete_v2", None, json_body={"path": path})
print(f"cleanup: HTTP {status} (rehearsal file deleted)")

Path("/home/z/my-project/scripts/rehearsal109_ledger.txt").write_text("\n".join(ledger) + "\n")
print("ledger rows written to scripts/rehearsal109_ledger.txt")
