#!/usr/bin/env python3
"""Post-private-flip audit (2026-11-11): quota burn, open issues, nightly, protection.

Private-tier billing: linux x1, windows x2, macos x10 against ~2000 min/month.
Token read from .secrets/gh_token, never printed.
"""
import json
import math
import os
import urllib.request
from datetime import datetime, timedelta, timezone

REPO = "kahlanhamza/IUM"
TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()


def api(path):
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def minutes(a, b):
    fmt = "%Y-%m-%dT%H:%M:%SZ"
    delta = datetime.strptime(b, fmt) - datetime.strptime(a, fmt)
    return math.ceil(delta.total_seconds() / 60)


def main():
    since = (datetime.now(timezone.utc) - timedelta(days=30)).strftime("%Y-%m-%dT%H:%M:%SZ")
    runs, page = [], 1
    while page <= 3:
        batch = api(f"/repos/{REPO}/actions/runs?per_page=100&created=>{since}&page={page}")
        runs.extend(batch.get("workflow_runs", []))
        if len(batch.get("workflow_runs", [])) < 100:
            break
        page += 1

    os_min = {"linux": 0, "windows": 0, "macos": 0}
    jobs_seen = 0
    for run in runs:
        jobs = api(f"/repos/{REPO}/actions/runs/{run['id']}/jobs?per_page=100").get("jobs", [])
        for job in jobs:
            if job.get("conclusion") in (None, "skipped"):
                continue
            runner = (job.get("runner_name") or "") or ",".join(job.get("labels") or [])
            bucket = "macos" if "macos" in runner or "mac-" in runner else (
                "windows" if "windows" in runner else "linux")
            os_min[bucket] += minutes(job["started_at"], job["completed_at"])
            jobs_seen += 1

    mult = {"linux": 1, "windows": 2, "macos": 10}
    quota_units = sum(os_min[k] * mult[k] for k in os_min)
    print(f"runs(30d)={len(runs)} jobs_billed={jobs_seen}")
    print(f"raw_minutes: linux={os_min['linux']} windows={os_min['windows']} macos={os_min['macos']}")
    print(f"quota_units_last30d={quota_units} (budget ~2000/month)")

    issues = api(f"/repos/{REPO}/issues?state=open&per_page=50")
    real = [i for i in issues if "pull_request" not in i]
    print(f"open_issues={len(real)}")
    for i in real:
        print(f"  #{i['number']} {i['title'][:90]}")

    nightly = api(f"/repos/{REPO}/actions/runs?workflow=nightly.yml&per_page=3")
    for r in nightly.get("workflow_runs", []):
        print(f"nightly {r['run_number']} {r['created_at'][:10]} {r['conclusion']}")

    prot = api(f"/repos/{REPO}/branches/main/protection")
    rsc = prot.get("required_status_checks") or {}
    print(f"protection: required_checks={len(rsc.get('contexts', []))} "
          f"enforce_admins={prot.get('enforce_admins', {}).get('enabled') if isinstance(prot.get('enforce_admins'), dict) else prot.get('enforce_admins')} "
          f"allow_force_pushes={(prot.get('allow_force_pushes') or {}).get('enabled')}")


if __name__ == "__main__":
    main()
