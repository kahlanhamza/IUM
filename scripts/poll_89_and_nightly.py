#!/usr/bin/env python3
"""Poll CI (head a1589cc) and the nightly rerun until both reach a verdict."""
import json
import time
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

HDR = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"}


def get(url: str):
    req = urllib.request.Request(url, headers=HDR)
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def runs_for(head: str):
    return get(
        f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs?head_sha={head}&per_page=20"
    )["workflow_runs"]


TARGET = "a1589cc"
NIGHTLY_RUN = 42468487548
seen = set()
for i in range(45):
    lines = []
    try:
        for r in runs_for(TARGET):
            key = (r["id"], r["run_attempt"])
            if key in seen:
                continue
            seen.add(key)
            lines.append(
                f"{r['name']} run {r['id']} attempt {r['run_attempt']}: {r['status']} / {r['conclusion']}"
            )
        # nightly rerun: same run id, new attempt
        n = get(f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs/{NIGHTLY_RUN}")
        key = (n["id"], n["run_attempt"])
        if key not in seen:
            seen.add(key)
        lines.append(
            f"nightly rerun run {n['id']} attempt {n['run_attempt']}: {n['status']} / {n['conclusion']}"
        )
    except Exception as e:  # noqa: BLE001
        lines.append(f"poll error: {e}")
    print(f"--- poll {i} ---", flush=True)
    for ln in lines:
        print(ln, flush=True)
    # done when nightly completed and CI completed
    try:
        n_done = False
        n = get(f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs/{NIGHTLY_RUN}")
        n_done = n["status"] == "completed"
        rs = runs_for(TARGET)
        ci_done = any(
            r["name"] == "ci" and r["status"] == "completed" for r in rs
        )
        if n_done and ci_done and i > 1:
            print("BOTH VERDICTS IN", flush=True)
            break
    except Exception:  # noqa: BLE001
        pass
    time.sleep(60)
