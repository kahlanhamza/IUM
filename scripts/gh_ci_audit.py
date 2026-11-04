#!/usr/bin/env python3
"""Audit CI runs, open issues and PRs for kahlanhamza/IUM. Token from .secrets (never printed)."""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def api(path: str):
    url = f"https://api.github.com/repos/{REPO}/{path}"
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-audit",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, json.loads(resp.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:600]


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) > 1 else "runs"

    if mode == "runs":
        status, runs = api("actions/runs?per_page=12")
        if status != 200:
            print(f"HTTP {status}: {runs}")
            return 1
        for r in runs["workflow_runs"]:
            print(f"#{r['run_number']:>3}  {r['conclusion'] or r['status']:<10} {r['head_sha'][:7]}  "
                  f"{r['head_branch']:<5} {r['name']:<28} {r['created_at']}  id={r['id']}")
        return 0

    if mode == "jobs":
        run_id = sys.argv[2]
        status, data = api(f"actions/runs/{run_id}/jobs?per_page=20")
        if status != 200:
            print(f"HTTP {status}: {data}")
            return 1
        for j in data["jobs"]:
            print(f"job: {j['name']:<40} {j['conclusion'] or j['status']}")
            for s in j["steps"]:
                if s["conclusion"] not in ("success", "skipped", None):
                    print(f"    step FAILED: {s['name']} ({s['conclusion']})")
        return 0

    if mode == "issues":
        status, issues = api("issues?state=open&per_page=100")
        if status != 200:
            print(f"HTTP {status}: {issues}")
            return 1
        for i in issues:
            kind = "PR" if "pull_request" in i else "issue"
            labels = ",".join(l["name"] for l in i["labels"])
            print(f"#{i['number']:>3} [{kind}] {i['title']}  ({labels})")
        return 0

    print("usage: gh_ci_audit.py runs|jobs <run_id>|issues")
    return 2


if __name__ == "__main__":
    sys.exit(main())
