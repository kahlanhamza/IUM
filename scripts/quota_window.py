#!/usr/bin/env python3
"""Billed-minute estimate for the PRIVATE-period runs (quota drain).

Private-repo billing multipliers: linux x1, windows x2, macos x10.
Only runs created in the given window count (public-repo runs are free).
Skipped jobs don't bill. Job-level ceil-minutes summed.
"""
import json
import math
import sys
import urllib.request
from datetime import datetime
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
REPO = "kahlanhamza/IUM"
FMT = "%Y-%m-%dT%H:%M:%SZ"


def api(path):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        headers={"Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def billed_minutes(window_start: str, window_end: str) -> dict:
    os_min = {"linux": 0, "windows": 0, "macos": 0}
    runs, page = [], 1
    while True:
        batch = api(f"actions/runs?per_page=100&page={page}")
        rs = batch["workflow_runs"]
        runs.extend(rs)
        if len(rs) < 100 or page >= 5:
            break
        page += 1
    for r in runs:
        if not (window_start <= r["created_at"][:19] < window_end):
            continue
        for j in api(f"actions/runs/{r['id']}/jobs?per_page=100")["jobs"]:
            if j["conclusion"] in (None, "skipped"):
                continue
            runner = (j.get("runner_name") or "") + "," + ",".join(j.get("labels") or [])
            bucket = ("macos" if ("macos" in runner or "mac-" in runner)
                      else "windows" if "windows" in runner else "linux")
            d = datetime.strptime(j["completed_at"], FMT) - datetime.strptime(j["started_at"], FMT)
            os_min[bucket] += max(1, math.ceil(d.total_seconds() / 60))
    mult = {"linux": 1, "windows": 2, "macos": 10}
    units = sum(os_min[k] * mult[k] for k in os_min)
    return {"raw": os_min, "units": units}


if __name__ == "__main__":
    start, end = sys.argv[1], sys.argv[2]
    r = billed_minutes(start, end)
    print(f"window {start} .. {end}")
    print(f"  raw minutes: linux={r['raw']['linux']} windows={r['raw']['windows']} macos={r['raw']['macos']}")
    print(f"  private-tier quota units: {r['units']} (free tier ~2000/month)")
