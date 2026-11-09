#!/usr/bin/env python3
"""Diagnose issue-#95 lane failures: print the failing steps' log regions.

Usage: python3 scripts/diag_95.py <run_id> <job_name_substring> <grep1,grep2,...>
"""
import json
import sys
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}
RUN = sys.argv[1]
JOB_SUB = sys.argv[2]
GREPS = [g.lower() for g in (sys.argv[3] if len(sys.argv) > 3 else "error").split(",")]

jobs = json.loads(
    urllib.request.urlopen(
        urllib.request.Request(
            f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs/{RUN}/jobs?per_page=20",
            headers=H,
        ),
        timeout=30,
    ).read().decode()
)
target = None
for j in jobs["jobs"]:
    if j["conclusion"] == "failure" and JOB_SUB in j["name"]:
        target = j
        break
if not target:
    print(f"no failing job matching {JOB_SUB!r}")
    sys.exit(1)
print("job:", target["name"], target["id"])
for s in target["steps"]:
    if s["conclusion"] not in ("success", "skipped", None):
        print("  failed step:", s["name"])

url = f"https://api.github.com/repos/kahlanhamza/IUM/actions/jobs/{target['id']}/logs"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


opener = urllib.request.build_opener(NoRedirect)
try:
    opener.open(urllib.request.Request(url, headers=H), timeout=60)
    print("expected a 302, got none")
    sys.exit(1)
except urllib.error.HTTPError as e:
    if e.code not in (301, 302, 303, 307):
        raise
    blob = e.headers["Location"]
with urllib.request.urlopen(blob, timeout=120) as r:
    log = r.read().decode("utf-8", "replace")

lines = log.splitlines()
hits = [ln for ln in lines if any(g in ln.lower() for g in GREPS)]
print(f"--- {len(hits)} matching lines (last 40):")
print("\n".join(hits[-40:]))
