#!/usr/bin/env python3
"""File milestone M8 (absolute speed) + its issues, idempotently.

Owner mandate 2026-11-04: "let's implement what we need to implement so we
get absolute speed upload." The ladder is docs/07_Speed_Architecture.md §3
(six rungs, each independently shippable). EXECUTED 2026-11-04: milestone #7,
issues #48–#53 created (this script is the record; re-run is a no-op —
existing titles are skipped).

Repo token comes from .secrets/gh_token (never printed, never committed).
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path(".secrets/gh_token").read_text().strip()

MILESTONE_TITLE = "M8 — Absolute speed: the prober drives, the fleet adapts, the lanes multiply"
MILESTONE_DESC = (
    "docs/07 §3 ladder, turned into code. The law: no algorithm exceeds the sum of the "
    "summed bearers. M7 measured the engine at 99%+ of wire and built the prober; M8 wires "
    "the prober into planning (#48), adapts the fleet live (AIMD, #49), unlocks tus "
    "parallelism via concatenation (#50), sums real bearers across interfaces (#51), "
    "repairs loss without round-trips (#52), and shrinks bytes on cause-D links (#53)."
)

ISSUE_TITLES = [
    "M8#1 Wire the link prober into the manager: profile journaled at probe time, planner obeys the measured piece + fleet seed",
    "M8#2 AIMD fleet control: additive increase, multiplicative decrease, plateau stop — the 54x swing answered live",
    "M8#3 tus concatenation lane: the server-advertised path to parallel tus (prober + AIMD go live on the tunnel lane)",
    "M8#4 Multi-path bonding: bind workers to interfaces, sum the real bearers (SO_BINDTODEVICE scheduler)",
    "M8#5 FEC lane: RaptorQ parity repairs loss without round-trips (RFC 6330)",
    "M8#6 Cause-D byte reduction: entropy-gated zstd transfer mode with honest destination semantics",
]


def api(method: str, path: str, payload: dict | None = None):
    url = f"https://api.github.com/repos/{REPO}/{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-file-m8",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            body = resp.read()
            return resp.status, (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:500]


def main() -> int:
    status, milestones = api("GET", "milestones?state=all")
    if status != 200:
        print(f"milestone list failed: {status} {milestones}")
        return 1
    mnum = next((m["number"] for m in milestones if m["title"].startswith("M8 —")), None)
    print(f"milestone: #{mnum} (M8)")

    status, existing = api("GET", "issues?state=all&per_page=100")
    if status != 200:
        print(f"issue list failed: {status} {existing}")
        return 1
    titles = {i["title"]: i["number"] for i in existing if isinstance(existing, list)}
    for want in ISSUE_TITLES:
        got = titles.get(want)
        print(f"issue: {'#' + str(got) if got else 'MISSING'} — {want[:70]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
