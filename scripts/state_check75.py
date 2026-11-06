"""One-page repo truth: #75/#74 state, comments, CI runs, origin/main, tree check."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}
REPO = "https://api.github.com/repos/kahlanhamza/IUM"


def get(url):
    return json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=H), timeout=30).read().decode())


# 1. origin/main
main = get(f"{REPO}/git/ref/heads/main")
print("origin/main:", main["object"]["sha"])

# 2. issues state
for num in (74, 75):
    it = get(f"{REPO}/issues/{num}")
    print(f"#{num} [{it['state']}] {it['title'][:80]}")

# 3. #75 comments (how many, last one's first line)
comments = get(f"{REPO}/issues/75/comments")
print("#75 comments:", len(comments))
for c in comments[-3:]:
    first = [ln for ln in c["body"].splitlines() if ln.strip()][:1]
    print(f"  - {c['id']} {c['created_at']}: {first[0][:100] if first else '(empty)'}")

# 4. recent CI runs
runs = get(f"{REPO}/actions/runs?per_page=6")
for r in runs["workflow_runs"]:
    print(f"run {r['id']} [{r['status']}/{r['conclusion']}] head={r['head_sha'][:7]} '{r['head_commit']['message'][:60]}'")

# 5. open issues list
open_issues = get(f"{REPO}/issues?state=open&per_page=20")
print("OPEN:", [(i["number"], i["title"][:50]) for i in open_issues if "pull_request" not in i])
