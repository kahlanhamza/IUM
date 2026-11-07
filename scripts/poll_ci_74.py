"""Poll CI on c170ded until completed (bounded ~9.5 min per call)."""
import json
import time
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}
REPO = "https://api.github.com/repos/kahlanhamza/IUM"
SHA = "c170ded"


def find_run():
    runs = json.loads(
        urllib.request.urlopen(urllib.request.Request(f"{REPO}/actions/runs?per_page=10", headers=H), timeout=30).read().decode()
    )
    for r in runs["workflow_runs"]:
        if r["head_sha"].startswith(SHA):
            return r
    return None


deadline = time.time() + 430
run = find_run()
while time.time() < deadline:
    if run is None:
        time.sleep(20)
        run = find_run()
        continue
    r = json.loads(
        urllib.request.urlopen(urllib.request.Request(f"{REPO}/actions/runs/{run['id']}", headers=H), timeout=30).read().decode()
    )
    if r["status"] == "completed":
        print("RUN", r["id"], "COMPLETED:", r["conclusion"], flush=True)
        jobs = json.loads(
            urllib.request.urlopen(urllib.request.Request(f"{REPO}/actions/runs/{r['id']}/jobs", headers=H), timeout=30).read().decode()
        )
        for j in jobs["jobs"]:
            print(f"  {j['name']}: {j['conclusion']}", flush=True)
        break
    print(f"run {run['id']} still {r['status']}...", flush=True)
    time.sleep(45)
else:
    print("STILL RUNNING after the window", flush=True)
