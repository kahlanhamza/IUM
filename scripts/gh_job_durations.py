#!/usr/bin/env python3
"""Print per-job timing of a run to distinguish billing-block failures (instant) from test failures."""
import json
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
HDR = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-audit",
}

run_id = sys.argv[1]
req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/actions/runs/{run_id}/jobs?per_page=100", headers=HDR
)
with urllib.request.urlopen(req) as resp:
    jobs = json.loads(resp.read())

print(f"run {run_id}  jobs={len(jobs['jobs'])}")
for j in jobs["jobs"]:
    st, ct = j.get("started_at"), j.get("completed_at")
    dur = "—"
    if st and ct:
        t0 = datetime.fromisoformat(st.replace("Z", "+00:00"))
        t1 = datetime.fromisoformat(ct.replace("Z", "+00:00"))
        dur = f"{(t1 - t0).total_seconds():.0f}s"
    nsteps = len(j.get("steps") or [])
    step_done = sum(1 for s in (j.get("steps") or []) if s.get("conclusion"))
    print(
        f"  {j['name']:<30} status={j['status']:<10} concl={str(j['conclusion']):<10} "
        f"dur={dur:>6} steps={step_done}/{nsteps} run_at={st}"
    )
