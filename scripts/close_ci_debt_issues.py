#!/usr/bin/env python3
"""Close #35 and #37 with evidence comments — run #75 is the first all-green
CI since #61 (STATUS.md's plan: '#35 + #37 — close when this push's CI is green')."""
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


comment_35 = (
    "**GREEN on CI — run #75** (HEAD `dd046a7`): the `app-e2e` job passed — "
    "8/8 jobs green, the first all-green run since #61.\n\n"
    "What ran, through the REAL shell (tauri-driver → WebKitWebDriver → WebKitGTK "
    "under xvfb, real engine, real loopback servers):\n"
    "- **Leg A — tus lifecycle**: create → live percent climb (per-PATCH server acks) "
    "→ pause → resume → Done badge → inspector via real IPC (Σ bytes_confirmed == "
    "source size, every chunk uploaded/verified) → server-truth: recorder's tus "
    "session holds all 24 MiB, HEAD `upload-offset` == source size.\n"
    "- **Leg B — one-shot cancel**: create → running → cancel mid-transfer → "
    "Canceled badge → recorder holds NOTHING (`received_bytes == 0`), HEAD → 404.\n\n"
    "Failure chain this week, each fixed by its own commit and proven by the next "
    "run's wire truth: #65 suite lands → #66 cold-boot patience → #67 boot dump → "
    "#68 custom-protocol (shell served devUrl) → #69 async commands (abort at click) "
    "→ #70 tus adapter for leg A (one-shot percent contradiction) → #71 captured "
    "runtime (drive() aborted) → #72 capabilities (events ACL-denied, frozen row) → "
    "#73/#74 row addressing (newest-first list) → `dd046a7` record-at-boot (a perfect "
    "'holds nothing' cancel ending was mistaken for a missing server).\n\n"
    "Evidence: run 41726278945, job `app-e2e` ⇒ success; recorder record A "
    "`{tus_sessions: 1, tus_bytes: 25165824}`. Closes #35."
)

comment_37 = (
    "**CI debt cleared — run #75 GREEN** (8/8 jobs: fmt, core ×3 OS, ium-app ×3 OS, "
    "app-e2e), first all-green since #61.\n\n"
    "Ledger of every red in #62–#74 and its fix:\n"
    "- #62: `core (macos)` concurrency-cancel phantom (all real jobs green) → "
    "`cancel-in-progress: false` — cannot recur.\n"
    "- #63: windows clippy `-D warnings` (rlimit drill not unix-gated) → file-level "
    "`#![cfg(unix)]` with the reason in a comment; ubuntu tus-wire lane → #33/#37 "
    "wire facts (early-reject PATCH, three-way HEAD probe).\n"
    "- #64: tus-wire lane residue → absorbed by the #65 commit's adapter work.\n"
    "- #65–#74: the new app-e2e leg's own seam-by-seam shakedown (boot patience, "
    "boot dump, custom-protocol, async commands, tus leg A, runtime handle, ACL "
    "capabilities, row-by-content addressing, recorder record-at-boot).\n"
    "- Verification discipline this session: gates on the COMMITTED tree "
    "(`scripts/gate.sh`), plus a wire-level smoke of the final fix "
    "(boot-record exists + HEAD 404 pre-PUT + aborted-PUT leaves zeros).\n\n"
    "Rule honored the hard way: no new CI was pushed while a failure stood "
    "unaddressed — one fix, verified locally, one push, green. Closes #37."
)

api("POST", "/issues/35/comments", {"body": comment_35})
api("PATCH", "/issues/35", {"state": "closed"})
api("POST", "/issues/37/comments", {"body": comment_37})
api("PATCH", "/issues/37", {"state": "closed"})
print("#35:", api("GET", "/issues/35")["state"])
print("#37:", api("GET", "/issues/37")["state"])
