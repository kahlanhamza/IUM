"""Verify current repo state from the GitHub API: #66 lane text, #73, open issues, CI runs."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
API = "https://api.github.com"
HDRS = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-state-check",
}


def get(path):
    req = urllib.request.Request(f"{API}{path}", headers=HDRS)
    with urllib.request.urlopen(req, timeout=30) as r:
        data = json.loads(r.read().decode())
        link = r.headers.get("Link", "")
        return data, link


# 1. Issue #66 body — the product bar (read back, never guess)
issue66, _ = get("/repos/kahlanhamza/IUM/issues/66")
body = issue66["body"] or ""
print("=== #66:", issue66["title"], "| state:", issue66["state"], "===")
lines = body.splitlines()
# print the lane 7 (telemetry) section and the lane list
in_l7 = False
for i, ln in enumerate(lines):
    low = ln.lower()
    if "lane 7" in low or "telemetry" in low:
        in_l7 = True
    if in_l7:
        print(ln)
        if in_l7 and i > 0 and (ln.startswith("###") or ln.startswith("## ")) and "lane 7" not in low and "telemetry" not in low:
            break

print()
print("=== open issues ===")
open_issues, link = get("/repos/kahlanhamza/IUM/issues?state=open&per_page=50")
nums = [f"#{i['number']} {i['title'][:60]}" for i in open_issues if "pull_request" not in i]
print("\n".join(nums) if nums else "(none)")

print()
print("=== issue #73 ===")
try:
    i73, _ = get("/repos/kahlanhamza/IUM/issues/73")
    print(f"#73 state={i73['state']} title={i73['title']}")
    print("comments:", i73["comments"])
except Exception as e:
    print("no #73:", e)

print()
print("=== recent CI runs ===")
runs, _ = get("/repos/kahlanhamza/IUM/actions/runs?per_page=8")
for r in runs["workflow_runs"]:
    print(r["id"], r["head_sha"][:7], r["status"], r["conclusion"], "|", r["name"], "|", r["head_branch"], "|", r["created_at"])
