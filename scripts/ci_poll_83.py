#!/usr/bin/env python3
"""CI verdict poll for the #83 push (50e3765) — REST only, no cron."""
import json
import time
import urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()
REPO = "kahlanhamza/IUM"
SHA = "50e3765"


def get(path):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        headers={"Authorization": f"Bearer {TOK}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def head_sha():
    return get("commits/main")["sha"][:7]


deadline = time.time() + 20 * 60
while time.time() < deadline:
    runs = get(f"actions/runs?head_sha={head_sha()}&per_page=5")
    for run in runs.get("workflow_runs", []):
        if run["head_sha"].startswith(SHA) or head_sha() == SHA:
            status = run["status"]
            if status == "completed":
                print(f"run {run['id']} ({run['name']}): {run['conclusion']}  {run['html_url']}")
    names = {r["name"]: r for r in runs.get("workflow_runs", []) if r["head_sha"].startswith(SHA) or head_sha() == SHA}
    if names and all(r["status"] == "completed" for r in names.values()):
        verdicts = {n: r["conclusion"] for n, r in names.items()}
        print("VERDICT:", verdicts)
        if all(v == "success" for v in verdicts.values()):
            print("CI ALL GREEN")
            break
        print("CI NOT GREEN — inspect")
        break
    time.sleep(45)
else:
    print("TIMEOUT — CI still running after 20 min")
