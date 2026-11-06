"""Fetch the failing ium-app job log and print the error region."""
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
RUN = sys.argv[1] if len(sys.argv) > 1 else "42286137461"
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
    if j["conclusion"] == "failure" and "ubuntu" in j["name"]:
        target = j
        break
if not target:
    print("no failing ubuntu job found")
    sys.exit(1)
print("job:", target["name"], target["id"])
url = f"https://api.github.com/repos/kahlanhamza/IUM/actions/jobs/{target['id']}/logs"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


try:
    # The logs endpoint 302s to a signed blob URL; the token must NOT ride
    # along to the blob store — follow the redirect ourselves, unauthenticated.
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
    keep = [
        i
        for i, ln in enumerate(lines)
        if "error" in ln.lower() or "warning: unused" in ln.lower()
    ]
    shown = set()
    for i in keep[:40]:
        for k in range(max(0, i - 2), min(len(lines), i + 6)):
            if k not in shown:
                print(lines[k])
                shown.add(k)
        print("  ---")
except urllib.error.HTTPError as e:
    print("log fetch failed:", e)
