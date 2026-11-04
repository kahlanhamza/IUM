#!/usr/bin/env python3
"""Fetch the log tail of a failed job (by run id + name substring). Token never printed."""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
HDR = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-audit",
}


def get(url: str, binary: bool = False):
    req = urllib.request.Request(url, headers=HDR)
    with urllib.request.urlopen(req) as resp:
        data = resp.read()
    return data if binary else json.loads(data)


def main() -> int:
    run_id = sys.argv[1]
    needle = sys.argv[2] if len(sys.argv) > 2 else "core (windows"
    jobs = get(f"https://api.github.com/repos/{REPO}/actions/runs/{run_id}/jobs?per_page=20")
    targets = [j for j in jobs["jobs"] if needle in j["name"]]
    if not targets:
        print(f"no job matching {needle!r}; jobs: {[j['name'] for j in jobs['jobs']]}")
        return 2
    job = targets[0]
    print(f"job: {job['name']}  conclusion={job['conclusion']}  id={job['id']}")
    try:
        # The logs endpoint 302s to a signed Azure URL; re-sending the bearer
        # header to that host causes HTTP 401. Capture Location, fetch bare.
        import http.client

        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, req, fp, code, msg, headers, newurl):
                return None

        opener = urllib.request.build_opener(NoRedirect)
        req = urllib.request.Request(
            f"https://api.github.com/repos/{REPO}/actions/jobs/{job['id']}/logs", headers=HDR
        )
        try:
            opener.open(req)
            print("unexpected: no redirect")
            return 1
        except urllib.error.HTTPError as e:
            if e.code not in (301, 302, 303, 307):
                print(f"log fetch HTTP {e.code} (logs expire after ~90 days)")
                return 1
            location = e.headers["Location"]
        resp = urllib.request.urlopen(
            urllib.request.Request(location, headers={"User-Agent": "ium-audit"})
        )
        log = resp.read()
    except urllib.error.HTTPError as e:
        print(f"log fetch HTTP {e.code} (logs expire after ~90 days)")
        return 1
    text = log.decode("utf-8", "replace")
    lines = text.splitlines()
    print(f"log lines: {len(lines)}")
    # Failure-relevant lines: test harness verdicts, panics, assertion dumps
    keys = ("panicked", "FAILED", "failures:", "test result:", "assertion", "left:", "right:",
            "Error:", "error:", "warning: unused", "failures")
    hits = [ln for ln in lines if any(k in ln for k in keys)]
    print("--- matched lines (tail) ---")
    for ln in hits[-60:]:
        # strip timestamps/ISO prefixes GitHub prepends
        print(ln[:300])
    print("--- raw tail (last 30 lines) ---")
    for ln in lines[-30:]:
        print(ln[:300])
    return 0


if __name__ == "__main__":
    sys.exit(main())
