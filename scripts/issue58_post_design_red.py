#!/usr/bin/env python3
"""Post the #58 design note + RED evidence on the issue. Token never printed."""
import json
import os
import urllib.request

REPO = "kahlanhamza/IUM"
TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {"Authorization": f"Bearer {TOKEN}",
       "Accept": "application/vnd.github+json",
       "Content-Type": "application/json"}


def comment(body):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/issues/58/comments",
        data=json.dumps({"body": body}).encode(), headers=HDR, method="POST")
    with urllib.request.urlopen(req) as r:
        print("comment:", json.load(r)["html_url"])


DESIGN = """## Design note — D1–D6 (posted before the cut)

**D1 — ONE merged release section.** The pre-seeded `## [0.1.0] — 2026-11-02`
(M5's frozen-facade seed from #20) and `## [Unreleased]` (M5→M10 content)
become ONE `## [0.1.0] — 2026-11-05` section. Two sections for one version
would violate Keep-a-Changelog; the tag is one promise covering M1→M9 plus the
first M10 rung.

**D2 — the audit found 16 closed issues with NO changelog entry**
(#21, #31, #34, #35, #37, #38, #41–#46, #51, #52, #53, #54 — see the RED
evidence). The repo's own #20 rule cuts both ways: entries must cite issues,
and closed work must HAVE entries. The cut writes them now, sourced ONLY from
STATUS.md rows (every one has a row with commit hashes + measured evidence) —
never from memory.

**D3 — versions are already aligned** (verified this session): workspace
`0.1.0` (ium-core inherits), `ium-app` + `tauri.conf.json` `0.1.0`,
`ium-e2e` `0.1.0`. No bump, `Cargo.lock` untouched.

**D4 — tag + release.** Annotated tag `v0.1.0` ON the release commit; GitHub
Release created from the tag with honest-scope notes (what the engine
guarantees today, measured). CI green on the tagged commit is part of the
gate — verified by direct API queries after push (no cron, per owner
directive).

**D5 — the gate is honored.** #56 closed; #57 closed (board repair this
session — the landing commit `f63599c` used `refs #57` not `Closes #57`, so
GitHub never auto-closed; evidence comment `6663793835`, then explicit close);
suite **258/258** green on the committed tree, re-proven after recycle #7
(toolchain restored via `scripts/install-toolchain.sh`).

**D6 — structure of the merged section.** Milestone-chronological Added
buckets with the GitHub milestone titles verbatim (M1 engine → M3 transports →
M4 shell → M5 → stress-evidence tranche → M6 → M7 → M8 → M9–M10);
`Fixed` / `Changed` / `Process` keep Keep-a-Changelog semantics (new #37/#54
fixed entries; #21/#31/#38 process entries); `## [Unreleased]` remains as an
empty header for the next rung's entry. Existing entries are kept VERBATIM —
the merge reorders and fills gaps, it does not rewrite history."""

RED = """## RED — the failing release state at HEAD `f63599c`

Full evidence: `scripts/issue58_red_evidence.md` (committed with the cut).
Summary, all checked LIVE this session:

1. `git tag -l` → **empty** — the promised tag does not exist.
2. `GET /releases` → **[]** — no GitHub release exists.
3. CHANGELOG cannot satisfy the gate: `[Unreleased]` carries M5→M10 while a
   stale `[0.1.0] — 2026-11-02` seed holds only M1–M4 (two sections, one
   version, neither is the release section).
4. **16 closed issues have no entry** (#21, #31, #34, #35, #37, #38, #41–#46,
   #51, #52, #53, #54) — the #20 rule's inverse violation; the cut writes
   them from STATUS rows.
5. The one green part: versions already aligned at 0.1.0 everywhere
   (workspace/ium-core/ium-app/tauri.conf.json/ium-e2e).
6. Gate preconditions verified: #56 + #57 closed, suite **258/258** green
   (re-proven after recycle #7)."""

comment(DESIGN)
comment(RED)
