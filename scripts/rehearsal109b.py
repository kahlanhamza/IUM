#!/usr/bin/env python3
"""#109 live wire-facts rehearsal v2 — dynamic, flushed, persisted.

v1 finding (live wire fact, 2026-11-12): append_v2 at a BEHIND offset
(0 when the server already held 4 MiB) returned HTTP 200, NOT the documented
`incorrect_offset` 400. This v2:
  - flushes every print (v1 lost stdout on the assert) and persists the
    session id to a file immediately;
  - after any surprising 200, probes the session's true server offset with a
    far-offset append (expected incorrect_offset -> correct_offset);
  - reconciles and finishes either way, verifying content_hash before
    declaring success.
The token is read from .secrets/dropbox_rehearsal_token and never printed.
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


def log(*a):
    line = " ".join(str(x) for x in a)
    print(line, flush=True)
    ledger.append(line)


def mask(s: str, keep: int = 6) -> str:
    return s[:keep] + "…<masked>" if len(s) > keep else "<masked>"


def call(label: str, url: str, api_arg: dict | None, body: bytes | None = None,
         json_body: dict | None = None) -> tuple[int, dict | bytes]:
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
            data, status = r.read(), r.status
    except urllib.error.HTTPError as e:
        data, status = e.read(), e.code
    try:
        parsed: dict | bytes = json.loads(data) if data else {}
    except json.JSONDecodeError:
        parsed = data
    desc = json.dumps(parsed)[:300] if isinstance(parsed, dict) else f"{len(data)}B body"
    log(f"WIRE {label}: HTTP {status} {desc}")
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


# --- probe -------------------------------------------------------------------
status, acct = call("probe get_current_account", f"{RPC}/users/get_current_account", None)
assert status == 200, f"probe failed: {status} {acct}"
log(f"probe OK account_id={mask(acct.get('account_id', ''))}")

c1, c2, c3 = pattern(BLOCK), pattern(BLOCK), pattern(1024 * 1024)
total = c1 + c2 + c3
local_hash = content_hash(total)
log(f"payload {len(total)} B (4+4+1 MiB), local content_hash={local_hash}")

# --- start -------------------------------------------------------------------
status, res = call("start", f"{CONTENT}/upload_session/start", {"close": False})
assert status == 200 and "session_id" in res, f"start failed: {status} {res}"
sid = res["session_id"]
Path("/home/z/my-project/scripts/rehearsal109_sid.txt").write_text(sid)
log(f"start OK session_id={mask(sid)} (persisted)")

server_offset = 0


def append(label: str, offset: int, chunk: bytes) -> dict | bytes | None:
    """Returns realign dict if the server answered incorrect_offset, else None."""
    global server_offset
    status, res = call(label, f"{CONTENT}/upload_session/append_v2",
                       {"cursor": {"session_id": sid, "offset": offset}, "close": False}, chunk)
    err = res.get("error", {}) if isinstance(res, dict) else {}
    tagged = err.get(".tag") == "incorrect_offset" or "incorrect_offset" in err
    if status in (400, 409) and tagged:
        # live wire shape (2026-11-12): {"error": {".tag": "incorrect_offset",
        # "correct_offset": N}, ...} — correct_offset is a SIBLING of .tag
        correct = err["correct_offset"]
        log(f"  incorrect_offset: declared={offset} server correct_offset={correct}")
        return {"correct": correct}
    assert status == 200, f"{label} failed: {status} {res}"
    server_offset = max(server_offset, offset + len(chunk))
    return None


# --- append #1 @0 --------------------------------------------------------------
r = append("append#1 @0", 0, c1)
assert r is None, "unexpected realign on fresh session"

# --- append #2 at a DELIBERATELY BEHIND offset ---------------------------------
r = append("append#2 BEHIND @0 (deliberate)", 0, c2)
if r is not None:
    # documented behavior: realign to the server's correct_offset
    assert r["correct"] == BLOCK, f"unexpected correct_offset {r['correct']}"
    r = append(f"append#2 realigned @{r['correct']}", r["correct"], c2)
    assert r is None
    server_offset = BLOCK + len(c2)
    log("realign path: BEHIND offset rejected with incorrect_offset -> retried at correct offset")
else:
    # LIVE FINDING v1: behind-offset append answered 200. Probe the truth.
    log("BEHIND-offset append answered 200 — probing true server offset with far-offset append")
    r = append("probe far @100MiB", 100 * 1024 * 1024, c3)
    assert r is not None, "far-offset append unexpectedly accepted"
    true_offset = r["correct"]
    log(f"SERVER TRUTH: session holds {true_offset} B after behind-offset 200")
    if true_offset == 2 * BLOCK:
        log("semantics: behind-offset append was STORED at server offset (cursor ignored)")
    elif true_offset == BLOCK:
        log("semantics: behind-offset append was NO-OP (idempotent-retry tolerance)")
        r = append(f"append#2 proper @{BLOCK}", BLOCK, c2)
        assert r is None, "unexpected realign at proper offset"
        server_offset = 2 * BLOCK
    else:
        raise AssertionError(f"unexplained server offset {true_offset}")

# --- append #3 (tail) -----------------------------------------------------------
r = append(f"append#3 @{server_offset}", server_offset, c3)
if r is not None:
    log(f"  tail realign to {r['correct']}")
    r = append(f"append#3 realigned @{r['correct']}", r["correct"], c3)
    assert r is None
    server_offset = r["correct"] + len(c3) if r is None else server_offset
else:
    pass
log(f"appends done, journal believes server holds {server_offset} B")

# --- finish ---------------------------------------------------------------------
path = f"/ium-rehearsal-{int(time.time())}.bin"
status, meta = call("finish", f"{CONTENT}/upload_session/finish",
                    {"cursor": {"session_id": sid, "offset": server_offset},
                     "commit": {"path": path, "mode": "overwrite", "autorename": False}},
                    b"")
assert status == 200, f"finish failed: {status} {meta}"
server_hash = meta.get("content_hash")
log(f"finish OK name={meta.get('name')} size={meta.get('size')} server content_hash={server_hash}")

# --- verify-before-declare -------------------------------------------------------
match = server_hash == local_hash
log(f"VERIFY server==local content_hash -> {match}")
assert match, "content_hash MISMATCH — verify-before-declare refuses Done"

# --- cleanup ---------------------------------------------------------------------
status, _ = call("cleanup delete_v2", f"{RPC}/files/delete_v2", None, json_body={"path": path})
log(f"cleanup HTTP {status}")

Path("/home/z/my-project/scripts/rehearsal109_ledger.txt").write_text("\n".join(ledger) + "\n")
print("ledger rows written", flush=True)
