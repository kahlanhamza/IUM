#!/usr/bin/env python3
"""Poll the CI verdict on the #81 commit (45s intervals, no cron)."""
import json, time, urllib.request

TOK = open(".secrets/gh_token").read().strip()
SHA = "5debf2a"
FULL = None

def get(u):
    r = urllib.request.Request(u, headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(r, timeout=30) as resp:
        return json.load(resp)

# resolve the full sha
for c in get("https://api.github.com/repos/kahlanhamza/IUM/commits?per_page=3"):
    if c["sha"].startswith(SHA):
        FULL = c["sha"]
print("polling", FULL[:7])

for attempt in range(30):
    runs = get(f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs?head_sha={FULL}&per_page=5")
    rs = runs.get("workflow_runs", [])
    if rs:
        states = [(r["name"], r["status"], r["conclusion"]) for r in rs]
        done = all(s == "completed" for _, s, _ in states)
        print(f"[{attempt}] " + "; ".join(f"{n}: {s}/{c}" for n, s, c in states))
        if done:
            ok = all(c == "success" for _, _, c in states)
            print("VERDICT:", "ALL GREEN" if ok else "FAILURES PRESENT")
            break
    else:
        print(f"[{attempt}] no runs yet")
    time.sleep(45)
