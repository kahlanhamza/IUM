"""Post the RED evidence comment on #75."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
HDRS = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = Path("/home/z/my-project/scripts/issue75_red_evidence.md").read_text()
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/75/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**HDRS, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("comment posted:", c["id"])
