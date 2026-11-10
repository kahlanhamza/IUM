#!/usr/bin/env python3
"""Poll IUM workflow runs — timeline + optional per-run job verdicts.

Usage:
  python3 scripts/poll_runs.py                 # last 8 runs, one line each
  python3 scripts/poll_runs.py <run_id> [...]  # + job conclusions per run
"""
import json
import sys
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
REPO = "kahlanhamza/IUM"


def api(path):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        headers={"Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def main():
    runs = api("actions/runs?per_page=8")["workflow_runs"]
    for r in runs:
        print(f'#{r["run_number"]} {r["name"][:12]:12} {r["created_at"]} '
              f'{r["status"]}/{r["conclusion"]} attempt={r["run_attempt"]} '
              f'id={r["id"]}')
    for rid in sys.argv[1:]:
        jobs = api(f"actions/runs/{rid}/jobs?per_page=30")["jobs"]
        for j in jobs:
            print(f'  run {rid}  {j["name"]:44} {j["status"]}/{j["conclusion"]}')


if __name__ == "__main__":
    main()
