#!/usr/bin/env python3
"""GitHub issue tool for kahlanhamza/IUM: create / comment / close. Token never printed.

Usage:
  gh_post.py create "<title>" < body.md
  gh_post.py comment <issue_number> < body.md
  gh_post.py close <issue_number> "<closing note>"
"""
import json
import sys
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def api(method: str, path: str, payload: dict | None = None):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-audit",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read() or b"null")


def main() -> int:
    mode = sys.argv[1]
    if mode == "create":
        title = sys.argv[2]
        body = sys.stdin.read()
        issue = api("POST", "issues", {"title": title, "body": body})
        print(f"created #{issue['number']}: {issue['html_url']}")
        return 0
    if mode == "comment":
        num = sys.argv[2]
        body = sys.stdin.read()
        c = api("POST", f"issues/{num}/comments", {"body": body})
        print(f"comment id {c['id']}: {c['html_url']}")
        return 0
    if mode == "close":
        num = sys.argv[2]
        note = sys.argv[3] if len(sys.argv) > 3 else "closed"
        api("PATCH", f"issues/{num}", {"state": "closed"})
        c = api("POST", f"issues/{num}/comments", {"body": note})
        print(f"closed #{num}, closing comment id {c['id']}")
        return 0
    print("usage: gh_post.py create <title> < body | comment <n> < body | close <n> <note>")
    return 2


if __name__ == "__main__":
    sys.exit(main())
