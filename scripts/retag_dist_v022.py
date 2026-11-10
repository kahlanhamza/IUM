#!/usr/bin/env python3
"""IUM-dist: move release tag v0.2.2 to current main HEAD.

Wire fact: tag pointed at c9ef670 (pre-README-refresh) while main HEAD has the
theme-adaptive README + assets/logo-white.svg. The auto source zip is cut from
the tag -> it shipped the old dark-only README. Moving the ref regenerates the
archive; release assets are untouched (updater reads assets, not the tag).
"""
import json
import subprocess
import sys
import time
import urllib.request

REPO = "kahlanhamza/IUM-dist"
TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
}


def api(method: str, path: str, payload: dict | None = None, expect: int = 200):
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        headers={**HDR, **({"Content-Type": "application/json"} if payload else {})},
        method=method,
    )
    try:
        with urllib.request.urlopen(req) as r:
            body = r.read()
            return r.status, (json.loads(body) if body else {})
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:200]


def main() -> None:
    # 1. full sha of main
    _, br = api("GET", f"/repos/{REPO}/branches/main")
    head = br["commit"]["sha"]
    print("main HEAD:", head[:12])

    # 2. delete the stale tag ref
    code, _ = api("DELETE", f"/repos/{REPO}/git/refs/tags/v0.2.2", expect=204)
    print("delete old ref:", code)

    # 3. recreate at main HEAD
    code, res = api("POST", f"/repos/{REPO}/git/refs",
                    {"ref": "refs/tags/v0.2.2", "sha": head})
    print("recreate ref:", code, res.get("object", {}).get("sha", "")[:12] if isinstance(res, dict) else res)

    # 4. wait, then verify the regenerated archive unauth
    time.sleep(8)
    url = "https://github.com/kahlanhamza/IUM-dist/archive/refs/tags/v0.2.2.zip"
    for attempt in range(3):
        subprocess.run(["curl", "-sL", "-o", "/tmp/src2.zip", url], check=True)
        out = subprocess.run(["unzip", "-l", "/tmp/src2.zip"], capture_output=True, text=True).stdout
        has_white = "logo-white.svg" in out
        ok = has_white
        print(f"archive attempt {attempt + 1}: logo-white.svg in zip -> {has_white}")
        if ok:
            break
        time.sleep(15)
    if not ok:
        print("DIE: archive did not regenerate", file=sys.stderr)
        sys.exit(1)

    # 5. the zip README must carry the picture block
    subprocess.run(["unzip", "-o", "-d", "/tmp/srcx", "/tmp/src2.zip"], capture_output=True)
    readme = open("/tmp/srcx/IUM-dist-0.2.2/README.md", encoding="utf-8").read()
    assert "prefers-color-scheme" in readme, "zip README lacks picture block"
    assert "assets/logo-white.svg" in readme, "zip README lacks white twin ref"
    print("zip README: picture block + white twin -> OK")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"DIE: {exc}", file=sys.stderr)
        sys.exit(1)
