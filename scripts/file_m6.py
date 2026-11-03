#!/usr/bin/env python3
"""Idempotent GitHub milestone/issue bootstrap for IUM milestone 5: M6 — Production readiness.

Reads the PAT from /home/z/my-project/.secrets/gh_token (never printed, never logged).
Safe to re-run: existing milestone/issues are detected by title and skipped.

Usage: python3 scripts/file_m6.py
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN_PATH = Path("/home/z/my-project/.secrets/gh_token")

MILESTONE = {
    "title": "M6: Production readiness — the probe generation",
    "description": (
        "The engine is feature-complete and reliability-hardened — but on TWO pinned servers, "
        "ONE impairment axis (bandwidth), minute-scale horizons, and fast CI links. M6 proves it "
        "against the realities a product meets: lossy/latent networks, server diversity, long "
        "horizons and resource limits, and the real app shell. Motivated by the boundary "
        "register (docs/08 §5): each probe class exercises seams for the FIRST time. "
        "DoD: every probe issue closed with wire evidence; the readiness verdict in STATUS.md "
        "upgraded from 'engine-hardened' to 'production-proven' per axis."
    ),
}

ISSUES = [
    {
        "title": "core: the proxy models bandwidth only — a real network also loses, delays, and resets",
        "labels": ["core", "ci"],
        "body": (
            "## Problem\n\n"
            "`throttle_proxy.py` impairs **bandwidth** (victim + uniform modes, ≤1 s relay "
            "granularity — #28). A real uplink (the owner's included) also: **drops packets** "
            "(loss → TCP retransmit multi-second stalls while the connection stays alive), adds "
            "**RTT** (100–300 ms — confirm latency per part), **jitters**, and **RESETS "
            "mid-body** (NAT idle timeouts, captive portals, tower handoffs). None of these are "
            "modeled, so eviction/retry/resplit behavior under them is UNPROBED.\n\n"
            "**Boundary register (docs/08 §5) — entities touched:** manager, eviction, retry, "
            "throttle_proxy harness. **Edges at risk:** manager ↔ eviction (windows and strike "
            "ratios were tuned on bandwidth stalls — a lossy-but-alive link must not be "
            "falsely evicted without strikes evidence, and an alive-but-stalling link must "
            "still be); manager ↔ retry (reset-mid-body must consume counted attempts, never "
            "spin). **Pinning:** RED first — the current proxy cannot produce any of the three "
            "impairments; each new behavior gets its own test + scenario assert.\n\n"
            "## Acceptance criteria\n\n"
            "- proxy gains `--loss PCT`, `--latency-ms N`, `--jitter-ms N`, `--reset-after-bytes N` "
            "(stdlib only, same relay discipline as #28 — no burst/silence artifacts)\n"
            "- scenario A (lossy): ~2% loss on a 2 MiB/s lane → upload completes byte-exact; "
            "retries absorbed; no false eviction of the alive link\n"
            "- scenario B (high-RTT): 250 ms RTT → resplit/eviction behave exactly per policy "
            "docs (measured-median path unaffected in the good case)\n"
            "- scenario C (mid-body RST): attempt fails → journaled counted retry → resume "
            "zero-re-upload → Done\n"
            "- `stress.yml` inputs extended (loss/latency CSV); one cloud run with the ladder\n\n"
            "## Out of scope\n\nPacket-capture replay (scapy-class tooling); QUIC-path impairments.\n\n"
            "## Refs\n docs/08 §5, #28 (relay discipline), #18/#25 (eviction seams)"
        ),
    },
    {
        "title": "bench: two servers are not an ecosystem — wire facts against a third S3/TUS implementation",
        "labels": ["ci", "tests"],
        "body": (
            "## Problem\n\nEvery S3 wire fact is pinned against SeaweedFS 4.51 (plus the Garage "
            "rehearsal); every TUS fact against tusd 2.10.1. Real users point at AWS S3, "
            "Backblaze B2, Wasabi, Nextcloud. The server-semantic defects found so far — "
            "numeric character references (`&#34;` → `InvalidPart`), #24's duplicate "
            "completed-but-unacknowledged PUT, #30's flush race — ALL lived on the "
            "adapter ↔ server-wire seam (docs/08 §3). Two servers cannot speak for an ecosystem.\n\n"
            "**Boundary register — entities touched:** adapter_s3, adapter_tus, harnesses. "
            "**Edges at risk:** the two adapter ↔ server-wire rows. **Pinning:** each wire-fact "
            "delta against the new server becomes a pinned test or an explicitly recorded "
            "honest skip — never silently absorbed.\n\n"
            "## Acceptance criteria\n\n"
            "- third S3 lane: `rclone serve s3` (pinned version, md5-verified download — same "
            "harness pattern as SeaweedFS) added to `scripts/s3-test.sh`; suite green against "
            "it or per-test honest skips WITH the wire-fact delta recorded on this issue\n"
            "- third TUS lane: a second independent TUS 1.0.0 server if one exists and is "
            "pinnable; otherwise the finding itself (\"ecosystem = tusd only\") recorded\n"
            "- optional owner-input lane (marked as such): real AWS S3 / B2 free-tier smoke if "
            "the owner supplies creds — never a CI dependency\n"
            "- any adapter change required by a delta goes through its own RED, own issue\n\n"
            "## Out of scope\n\nOAuth flows (accounts milestone); non-S3/TUS protocols.\n\n"
            "## Refs\n docs/08 §3 adapter↔server rows, #9 (server re-decision precedent), #24, #30"
        ),
    },
    {
        "title": "core: long horizon & resource honesty — hours at B/s, journal growth, RSS, ENOSPC",
        "labels": ["core", "ci"],
        "body": (
            "## Problem\n\nThe #28 ladder legs are timeboxed MINUTES. The owner's real link "
            "needs HOURS (30 MB @ 512 B/s ≈ 17 h). Unprobed at long horizon: journal (SQLite "
            "WAL) growth over tens of thousands of events, manager RSS boundedness overnight, "
            "and the sharpest one — **ENOSPC while the journal is being written**: the journal "
            "is truth (ADR-002), so what does the engine do when TRUTH cannot be written? "
            "Silent corruption or a loud, honest, recoverable failure — currently unknown.\n\n"
            "**Boundary register — entities touched:** journal, manager, harness. **Edges at "
            "risk:** journal ↔ everything (writes must fail loudly per Sprint-0 #2 symmetry — "
            "but ENOSPC during `transition()` before-execute is a new failure POINT); manager "
            "↔ chunk pool (resume after disk-free). **Pinning:** the ENOSPC drill test (RED: "
            "current suite has no ENOSPC coverage — that absence is the RED); bounded-growth "
            "assertions with pinned numbers.\n\n"
            "## Acceptance criteria\n\n"
            "- overnight B/s leg in the free cloud (timeboxed, progress telemetry, artifact): "
            "Done or honest timebox-cancel, journal replayable and state truthful afterwards\n"
            "- ENOSPC drill: journal on a small filesystem (loopback image, sudo on runners) → "
            "write failure is LOUD (no silent corruption, no false Done), job ends Paused/Failed "
            "honestly, resume after freeing space completes byte-exact\n"
            "- journal size and manager RSS pinned under a many-event soak (numbers recorded "
            "on the issue; bounds asserted in a test)\n\n"
            "## Out of scope\n\nJournal compaction/VACUUM policy (own issue if the soak shows need).\n\n"
            "## Refs\n ADR-002, #28 (ladder), Sprint-0 #2 (loud writes), docs/08 §3 journal rows"
        ),
    },
    {
        "title": "app: nobody has uploaded through the real shell — WebDriver e2e on the desktop lane",
        "labels": ["ci", "enhancement"],
        "body": (
            "## Problem\n\n`ium-app` compiles on 3 OS (#14) and UI logic is vitest-tested "
            "(#16) — but there is NO end-to-end proof of a human-visible upload: create job → "
            "watch live progress → pause/resume/cancel → Done badge, in the real Tauri shell. "
            "The engine is proven; the product's only surface is not. The owner's readiness "
            "question (\"does our engine ready? I don't think so\") cannot be answered 'yes' "
            "while the shell has never been driven.\n\n"
            "**Boundary register — entities touched:** ium-app shell, supervisor, manager. "
            "**Edges at risk:** shell ↔ supervisor command/event contract (job_updated "
            "watcher, inspect_chunks), the UI action-gating logic vs the engine state machine. "
            "**Pinning:** the e2e itself is the pin; any gap found files its own issue "
            "(register discipline — no drive-by fixes).\n\n"
            "## Acceptance criteria\n\n"
            "- tauri-driver (WebDriver) e2e on the ubuntu CI lane against the HTTP loopback "
            "recorder: real file in → create → live progress observed → pause → resume → "
            "Done badge; journal snapshot via inspect_chunks asserted from the test\n"
            "- if the lane cannot run headless, the finding is recorded honestly with the "
            "exact blocker and a minimal shell-smoke fallback (app boots, lists adapters, "
            "drives one job to Done) ships instead — either way the surface is driven once\n"
            "- cancel path in the UI: cancel button → Canceled badge → no server object (HEAD 404)\n\n"
            "## Out of scope\n\nVisual regression snapshots; Sprint-2 feature work (separate tranche).\n\n"
            "## Refs\n #14/#15/#16, docs/08 §5 'new surface' row, owner readiness question 2026-11-03"
        ),
    },
]


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
            "User-Agent": "ium-m6",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            body = resp.read()
            return resp.status, (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:400]


def main() -> int:
    if not TOKEN_PATH.exists() or not TOKEN_PATH.read_text().strip():
        print("BLOCKED: token missing in .secrets/gh_token")
        return 2

    status, milestones = api("GET", "milestones?state=all")
    milestone_number = None
    if status == 200 and milestones:
        milestone_number = next(
            (m["number"] for m in milestones if m["title"] == MILESTONE["title"]), None
        )
    if milestone_number is None:
        status, m = api("POST", "milestones", MILESTONE)
        milestone_number = m["number"] if status == 201 else None
    print(f"milestone: #{milestone_number} ({MILESTONE['title']})")

    status, existing = api("GET", "issues?state=all&per_page=100")
    existing_titles = {i["title"]: i["number"] for i in existing} if status == 200 else {}

    for issue in ISSUES:
        if issue["title"] in existing_titles:
            print(f"issue: EXISTS #{existing_titles[issue['title']]} — {issue['title']}")
            continue
        payload = {"title": issue["title"], "body": issue["body"], "labels": issue["labels"]}
        if milestone_number:
            payload["milestone"] = milestone_number
        status, created = api("POST", "issues", payload)
        if status == 201:
            print(f"issue: CREATED #{created['number']} — {issue['title']}")
        else:
            print(f"issue: FAILED http-{status} — {issue['title']}: {created}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
