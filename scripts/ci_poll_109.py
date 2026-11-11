#!/usr/bin/env python3
"""Poll the CI run for HEAD 273b073 (#109) until completion."""
import json
import time
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
SHA = (
    __import__("subprocess")
    .check_output(["git", "rev-parse", "HEAD"], text=True)
    .strip()
)


def api(path: str) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/{path}",
        headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


run_id = None
for attempt in range(30):
    runs = api(f"actions/runs?head_sha={SHA}").get("workflow_runs", [])
    if runs:
        run_id = runs[0]["id"]
        break
    time.sleep(10)

if not run_id:
    raise SystemExit("no CI run appeared for the HEAD within 5 minutes")

print(f"CI run {run_id} for {SHA}")
while True:
    run = api(f"actions/runs/{run_id}")
    if run["status"] != "completed":
        time.sleep(30)
        continue
    jobs = api(f"actions/runs/{run_id}/jobs")["jobs"]
    print(f"conclusion: {run['conclusion']}")
    for j in jobs:
        print(f"  {j['name']}: {j['conclusion']} ({j['completed_at']})")
    raise SystemExit(0 if run["conclusion"] == "success" else 1)
