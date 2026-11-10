#!/usr/bin/env python3
"""IUM-dist: theme-adaptive logomark in README.

Owner report: assets/logo.svg (ink #0F172A) is invisible on GitHub dark theme.
Fix = the white mono twin (design/logo-white.svg, ink #FFFFFF, same
owner-approved set03 geometry) + <picture> prefers-color-scheme switch.
"""
import base64
import json
import sys
import urllib.request

REPO = "kahlanhamza/IUM-dist"
TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
}


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        headers={**HDR, **({"Content-Type": "application/json"} if payload else {})},
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        body = r.read()
    return json.loads(body) if body else {}


def main() -> None:
    # 1. upload the white twin
    svg = open("/home/z/my-project/design/logo-white.svg", "rb").read()
    assert b"#FFFFFF" in svg and b"#0F172A" not in svg, "white twin sanity"
    put = api("PUT", f"/repos/{REPO}/contents/assets/logo-white.svg", {
        "message": "assets: add the white ink logomark (dark-surface twin of the set03 dart)",
        "content": base64.b64encode(svg).decode(),
    })
    print("uploaded assets/logo-white.svg @", put["content"]["sha"][:12])

    # 2. rewrite the README header img -> picture (theme adaptive)
    cur = api("GET", f"/repos/{REPO}/contents/README.md")
    text = base64.b64decode(cur["content"]).decode()
    old = ('  <img src="assets/logo.svg" width="96" alt="The IUM logomark '
           '— the dart: two smooth concentric arcs on one bow, round caps, '
           'one flat ink" />')
    new = ('  <picture>\n'
           '    <source media="(prefers-color-scheme: dark)" '
           'srcset="assets/logo-white.svg" />\n'
           '    <img src="assets/logo.svg" width="96" alt="The IUM logomark '
           '— the dart: two smooth concentric arcs on one bow, round caps, '
           'one flat ink" />\n'
           '  </picture>')
    assert text.count(old) == 1, "header img block not found exactly once"
    upd = api("PUT", f"/repos/{REPO}/contents/README.md", {
        "message": "readme: theme-adaptive logomark — white ink on dark, slate ink on light",
        "content": base64.b64encode(text.replace(old, new).encode()).decode(),
        "sha": cur["sha"],
    })
    print("README updated @", upd["commit"]["sha"][:12])


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"DIE: {exc}", file=sys.stderr)
        sys.exit(1)
