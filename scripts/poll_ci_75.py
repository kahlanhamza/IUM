"""Poll CI run 42286137461 until completed (bounded ~9.5 min per call)."""
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
RUN = "42291038005"
URL = f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs/{RUN}"
JOBS_URL = f"{URL}/jobs"

deadline = time.time() + 430
while time.time() < deadline:
    r = json.loads(
        urllib.request.urlopen(urllib.request.Request(URL, headers=H), timeout=30).read().decode()
    )
    if r["status"] == "completed":
        print("RUN COMPLETED:", r["conclusion"], flush=True)
        jobs = json.loads(
            urllib.request.urlopen(urllib.request.Request(JOBS_URL, headers=H), timeout=30).read().decode()
        )
        for j in jobs["jobs"]:
            print(f"  {j['name']}: {j['conclusion']}", flush=True)
        break
    print(f"still {r['status']}...", flush=True)
    time.sleep(45)
else:
    print("STILL RUNNING after the window", flush=True)
