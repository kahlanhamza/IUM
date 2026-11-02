#!/usr/bin/env python3
"""Post a comment to a GitHub issue using the token from .secrets/gh_token."""
import json
import sys
import urllib.request


def main() -> None:
    issue = sys.argv[1]
    body_file = sys.argv[2]
    with open("/home/z/my-project/.secrets/gh_token") as f:
        token = f.read().strip()
    with open(body_file) as f:
        body = f.read()
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/issues/{issue}/comments",
        data=json.dumps({"body": body}).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req) as resp:
        out = json.load(resp)
    print(f"comment id={out['id']} url={out['html_url']}")


if __name__ == "__main__":
    main()
