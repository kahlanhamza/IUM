#!/usr/bin/env python3
"""Resolve PR #38: superseded by main's #39 fix (940ddc9)."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
BASE = "https://api.github.com/repos/kahlanhamza/IUM"


def api(method: str, path: str, body: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"{BASE}{path}",
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
    )
    with urllib.request.urlopen(req) as r:
        data = r.read()
    return json.loads(data) if data else {}


body = (
    "Closing as **superseded** — the bug this PR fixes (main-thread `tokio::spawn` "
    "abort on the first Upload click) already landed on main through a more "
    "complete fix, and CI has proven the shell end-to-end since.\n\n"
    "## Why not merge\n"
    "- This PR made `create_job`/`resume_signal` `async` so the supervisor's "
    "`drive()` spawns inside a tokio context. Main's `940ddc9` (issue #39) fixed "
    "the same abort at the root: `Supervisor` captures a `tokio::runtime::Handle` "
    "at construction (`with_handle`), so **every** spawn site is legal — the two "
    "commands in this PR, `resume` re-drives, and any future caller — with two "
    "new regression tests (`drive_is_callable_off_the_runtime_thread` + the "
    "shell's construct-and-drive-off-runtime shape). Same commit also shipped "
    "the `cancel-in-progress: false` CI fix recorded on #39.\n"
    "- The panic-forensics grep this PR adds to `app-e2e.sh` is covered on main "
    "by `tail -200` (the #39 lesson row in STATUS.md), which is what actually "
    "captured the #70/#71 backtraces.\n"
    "- Proof the supersession is complete: **run #75 — 8/8 jobs green** "
    "(41726278945), including `app-e2e` driving the real shell through both "
    "legs (tus create/progress/pause/resume/done/inspect + one-shot "
    "cancel-no-object). The abort is gone.\n\n"
    "The diagnosis in this PR's description fed directly into #39's root-cause "
    "write-up — the analysis was the valuable part, and it is preserved on the "
    "issue. Branch `e2e-fix` deleted; the loop continues on main (issues #35 "
    "and #37 closed on run #75's evidence)."
)

api("POST", "/issues/38/comments", {"body": body})
api("PATCH", "/pulls/38", {"state": "closed"})
try:
    api("DELETE", "/git/refs/heads/e2e-fix")
    print("branch e2e-fix deleted")
except Exception as e:
    print("branch delete:", e)
p = api("GET", "/pulls/38")
print("PR #38 state:", p["state"], "| merged:", p.get("merged"))
