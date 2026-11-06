"""File issue #75 (Lane 7: the telemetry stance) with the design note as body."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
HDRS = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = Path("/home/z/my-project/scripts/issue75_design_note.md").read_text()
payload = {
    "title": (
        "The telemetry stance: consented, allowlisted, locally-inspectable "
        "diagnostics — the written privacy contract (product bar #66 lane 7)"
    ),
    "body": body,
    "labels": ["product-bar"],
}
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps(payload).encode(),
    headers={**HDRS, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    issue = json.loads(r.read().decode())
print("filed:", issue["number"], issue["state"], "|", issue["title"][:80])
print("url:", issue["html_url"])
