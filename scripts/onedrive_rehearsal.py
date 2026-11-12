#!/usr/bin/env python3
"""#110 live rehearsal — device-code pivot (NO redirect URI, NO port, NO paste race).

Phases:
  --start : POST /common/oauth2/v2.0/devicecode -> print verification_uri +
            user_code (the owner's ONE action: open the URI, type the code).
  --wait  : poll the token endpoint until authorization_pending clears
            (interval + slow_down honored, 15-min device-code window), save
            the token pair (600), then drive the FULL upload rehearsal:
            probe me/drive -> createUploadSession -> 2 x 5 MiB fragments +
            tail -> one DELIBERATE misaligned Content-Range (expect 416 +
            nextExpectedRanges, server-wins realign) -> correct final PUT
            (the atomic commit) -> metadata quickXorHash vs LOCAL hash ->
            cleanup DELETE. Every wire fact masked and appended to
            .secrets/onedrive_rehearsal_ledger.txt (LOCAL ONLY, 0600).

Tokens never print; no PII (no displayName / userPrincipalName / email).
"""
import base64
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from quickxorhash import quick_xor_hash  # noqa: E402

SECRETS = Path("/home/z/my-project/.secrets")
TOKF = SECRETS / "onedrive_rehearsal_token.json"
LEDGER = SECRETS / "onedrive_rehearsal_ledger.txt"
CLIENT_ID = (SECRETS / "onedrive_client_id").read_text().strip()
SCOPE = "offline_access Files.ReadWrite User.Read"
AUTH = "https://login.microsoftonline.com/common/oauth2/v2.0"
GRAPH = "https://graph.microsoft.com/v1.0"
CHUNK = 5 * 1024 * 1024          # the prescribed uniform block (16 x 320 KiB)
KI320 = 327_680


def post_form(url: str, data: dict) -> tuple[int, dict]:
    req = urllib.request.Request(url, data=urllib.parse.urlencode(data).encode(), headers={
        "Content-Type": "application/x-www-form-urlencoded",
    })
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode())
        except Exception:
            return e.code, {}


def graph(method: str, url: str, token: str, body: bytes | None = None,
          headers: dict | None = None) -> tuple[int, bytes | dict]:
    h = {"Authorization": f"Bearer {token}"}
    if headers:
        h.update(headers)
    req = urllib.request.Request(url, data=body, headers=h, method=method)
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            raw = r.read()
            try:
                return r.status, json.loads(raw)
            except Exception:
                return r.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, raw


def save_tokens(resp: dict) -> None:
    tok = {
        "access_token": resp["access_token"],
        "refresh_token": resp.get("refresh_token"),
        "expires_in": resp["expires_in"],
        "token_type": resp.get("token_type"),
        "scope": resp.get("scope", ""),
        "obtained_at": int(time.time()),
    }
    TOKF.write_text(json.dumps(tok) + "\n")
    TOKF.chmod(0o600)
    if not tok["refresh_token"]:
        print("FAIL: no refresh token — offline_access missing")
        sys.exit(1)
    a, r = tok["access_token"], tok["refresh_token"]
    print(f"tokens saved (600): access len={len(a)} head={a[:6]}…, refresh len={len(r)} head={r[:6]}…")
    print(f"expires_in={tok['expires_in']}s scope={tok['scope']}")


def start() -> None:
    st, resp = post_form(f"{AUTH}/devicecode", {"client_id": CLIENT_ID, "scope": SCOPE})
    if st != 200:
        print(f"devicecode FAILED: HTTP {st} {json.dumps(resp)[:300]}")
        sys.exit(1)
    print(f"verification_uri : {resp['verification_uri']}")
    print(f"user_code        : {resp['user_code']}")
    print(f"expires_in       : {resp['expires_in']}s  poll_interval={resp['interval']}s")


def wait_and_rehearse() -> None:
    st, resp = post_form(f"{AUTH}/devicecode", {"client_id": CLIENT_ID, "scope": SCOPE})
    if st != 200:
        print(f"devicecode FAILED: HTTP {st} {json.dumps(resp)[:300]}")
        sys.exit(1)
    uri, code = resp["verification_uri"], resp["user_code"]
    interval, deadline = resp["interval"], time.time() + resp["expires_in"]
    print(f"WAITING for authorization: open {uri} and type {code} (window {resp['expires_in']}s)")
    token_resp = None
    while time.time() < deadline:
        time.sleep(interval)
        st, tr = post_form(f"{AUTH}/token", {
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "client_id": CLIENT_ID,
            "device_code": resp["device_code"],
        })
        if st == 200:
            token_resp = tr
            break
        err = tr.get("error", "?")
        if err == "authorization_pending":
            continue
        if err == "slow_down":
            interval += 5
            continue
        print(f"poll FAILED: {err} {json.dumps(tr.get('error_description', ''))[:200]}")
        sys.exit(1)
    if not token_resp:
        print("device code window expired without authorization")
        sys.exit(1)
    save_tokens(token_resp)
    tok = json.loads(TOKF.read_text())
    rehearse(tok["access_token"])


def rehearse(tok: str) -> None:
    facts: list[str] = []

    def fact(line: str) -> None:
        facts.append(line)
        print(line)

    st, drive = graph("GET", f"{GRAPH}/me/drive", tok)
    fact(f"probe /me/drive -> {st} driveType={drive.get('driveType')} driveId={drive.get('id')}")
    q = drive.get("quota", {})
    fact(f"  quota total={q.get('total')} used={q.get('used')} state={q.get('state')}")
    if st != 200:
        sys.exit(1)

    ts = int(time.time())
    path = f"ium-rehearsal-110/rehearsal_{ts}.bin"
    total = 2 * CHUNK + 1024 * 1024 + 123
    data = bytes(i % 251 for i in range(total))
    st, sess = graph("POST", f"{GRAPH}/me/drive/root:/{path}:/createUploadSession", tok,
                     body=json.dumps({"item": {"@microsoft.graph.conflictBehavior": "replace"}}).encode(),
                     headers={"Content-Type": "application/json"})
    fact(f"createUploadSession -> {st} expiry={sess.get('expirationDateTime', '?')}")
    up = sess.get("uploadUrl")
    if st != 200 or not up:
        fact(f"  body: {json.dumps(sess)[:300]}")
        sys.exit(1)

    # fragment plan: 5 MiB, 5 MiB, 1 MiB+123 — every non-final a 320 KiB multiple
    plan = [(0, CHUNK), (CHUNK, 2 * CHUNK), (2 * CHUNK, total)]
    assert all((e - s) % KI320 == 0 for s, e in plan[:-1]), "non-final fragments must be 320 KiB multiples"
    assert plan[-1][1] - plan[-1][0] <= 60 * 1024 * 1024

    # chunk 0 — correct
    s, e = plan[0]
    st, r = graph("PUT", up, tok, body=data[s:e],
                  headers={"Content-Range": f"bytes {s}-{e - 1}/{total}"})
    fact(f"PUT chunk0 [{s},{e}) -> {st} nextExpectedRanges={r.get('nextExpectedRanges') if isinstance(r, dict) else r!r}")

    # DELIBERATE misalignment on chunk 1: claim an AHEAD offset (skip 1 MiB
    # into the fragment) — the wire must refuse and hand back the truth.
    s, e = plan[1]
    bogus_start = s + 1024 * 1024
    st, r = graph("PUT", up, tok, body=data[bogus_start:e],
                  headers={"Content-Range": f"bytes {bogus_start}-{e - 1}/{total}"})
    body = r if isinstance(r, dict) else {}
    fact(f"PUT chunk1 DELIBERATE-ahead [{bogus_start},{e}) -> {st}")
    realign_from = None
    if st == 416:
        ner = body.get("nextExpectedRanges", [])
        fact(f"  416 nextExpectedRanges={ner} (server wins — the #10 realign pattern)")
        if ner:
            realign_from = int(ner[0].split("-")[0])
    else:
        fact(f"  body: {json.dumps(body)[:300]}")

    # realigned chunk 1: send from the SERVER's expectation
    start1 = realign_from if realign_from is not None else s
    st, r = graph("PUT", up, tok, body=data[start1:e],
                  headers={"Content-Range": f"bytes {start1}-{e - 1}/{total}"})
    fact(f"PUT chunk1 realigned [{start1},{e}) -> {st} nextExpectedRanges={r.get('nextExpectedRanges') if isinstance(r, dict) else r!r}")

    # final fragment — the atomic commit (no separate finish call)
    s, e = plan[2]
    st, item = graph("PUT", up, tok, body=data[s:e],
                     headers={"Content-Range": f"bytes {s}-{e - 1}/{total}"})
    fact(f"PUT final [{s},{e}) -> {st} (201/200 = the commit itself)")
    if st not in (200, 201):
        fact(f"  body: {json.dumps(item)[:300]}")
        sys.exit(1)
    remote = (item.get("file") or {}).get("hashes", {}).get("quickXorHash")
    fact(f"driveItem id={item.get('id')} size={item.get('size')} quickXorHash={remote}")

    # verify gate: metadata read-back (what the adapter's verify fetches)
    st, meta = graph("GET", f"{GRAPH}/me/drive/root:/{path}:?$select=id,name,size,file", tok)
    remote2 = (meta.get("file") or {}).get("hashes", {}).get("quickXorHash")
    fact(f"metadata read-back -> {st} quickXorHash={remote2}")

    local = base64.b64encode(quick_xor_hash(data)).decode()
    ok = local == remote == remote2
    fact(f"LOCAL quickXorHash ={local}")
    fact(f"VERIFY {'PASS' if ok else 'FAIL'} — local == commit receipt == metadata read-back")

    st, _ = graph("DELETE", f"{GRAPH}/me/drive/root:/{path}:", tok)
    fact(f"cleanup DELETE -> {st}")

    LEDGER.write_text("\n".join(facts) + "\n")
    LEDGER.chmod(0o600)
    print(f"ledger: {LEDGER} ({len(facts)} rows, LOCAL ONLY)")


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "--start"
    if mode == "--start":
        start()
    elif mode == "--wait":
        wait_and_rehearse()
    elif mode == "--rehearse":
        rehearse(json.loads(TOKF.read_text())["access_token"])
    else:
        print("usage: onedrive_rehearsal.py [--start|--wait|--rehearse]")
        sys.exit(2)
