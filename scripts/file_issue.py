#!/usr/bin/env python3
"""File ONE GitHub issue for IUM from a JSON payload file.

Reads the PAT from .secrets/gh_token (never printed, never logged).
Payload JSON: {"title": str, "body": str, "labels": [str], "milestone": int?, "issue": int?}
  - "issue": optional existing issue number -> print it (idempotent re-run support is
    handled by title match against existing issues).

Usage: python3 scripts/file_issue.py payload.json
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN_PATH = Path("/home/z/my-project/.secrets/gh_token")


def api(method: str, path: str, payload: dict | None = None):
    url = f"https://api.github.com/repos/{REPO}/{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {TOKEN_PATH.read_text().strip()}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-loop",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            body = resp.read()
            return resp.status, (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:400]


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: file_issue.py <payload.json>")
        return 2
    spec = json.loads(Path(sys.argv[1]).read_text())

    status, existing = api("GET", "issues?state=all&per_page=100")
    if status == 200:
        for i in existing:
            if i["title"] == spec["title"]:
                print(f"issue: EXISTS #{i['number']} — {spec['title']}")
                return 0

    payload = {
        "title": spec["title"],
        "body": spec["body"],
        "labels": spec.get("labels", []),
    }
    if spec.get("milestone"):
        payload["milestone"] = spec["milestone"]
    status, created = api("POST", "issues", payload)
    if status == 201:
        print(f"issue: CREATED #{created['number']} — {spec['title']}")
        return 0
    print(f"issue: FAILED http-{status}: {created}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
