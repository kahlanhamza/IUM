#!/usr/bin/env python3
"""CI verdict poll for the #82 loop commit — 45s REST intervals, head_sha pinned."""
import json, sys, time, urllib.request

SHA = "87d111347db02aace7b5c94594becfe920bcbd3e"
TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()


def runs():
    r = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/commits/{SHA}/check-runs?per_page=100",
        headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    )
    return json.load(urllib.request.urlopen(r)).get("check_runs", [])


deadline = time.time() + 30 * 60
while time.time() < deadline:
    rs = runs()
    if rs:
        pending = [c["name"] for c in rs if c["status"] != "completed"]
        failed = [c["name"] for c in rs if c["conclusion"] not in ("success", "skipped", None) and c["status"] == "completed"]
        print(f"[{time.strftime('%H:%M:%S')}] {len(rs)} runs, pending={len(pending)} {pending or ''}", flush=True)
        if failed:
            print("CI VERDICT: FAIL —", failed, flush=True)
            sys.exit(1)
        if not pending:
            print("CI VERDICT: GREEN — all check runs completed:", flush=True)
            for c in rs:
                print(f"  {c['name']}: {c['conclusion']}", flush=True)
            sys.exit(0)
    else:
        print(f"[{time.strftime('%H:%M:%S')}] no check runs yet", flush=True)
    time.sleep(45)
print("CI VERDICT: TIMEOUT", flush=True)
sys.exit(2)
