#!/usr/bin/env python3
"""#57 board repair: the landing commit f63599c used `refs #57` (not
`Closes #57`), so GitHub never auto-closed the issue even though the work
landed, gated (258/258) and the STATUS row was pushed. Post the evidence
comment and close the issue as completed. Token never printed."""
import json
import os
import sys
import urllib.request

REPO = "kahlanhamza/IUM"
TOKEN = open(os.path.expanduser(
    "/home/z/my-project/.secrets/gh_token")).read().strip()

BODY = """Board repair, recorded honestly: the rung-2 landing commit `f63599c`
("feat(core): the prober covers the S3 lane — measured profiles for multipart
uploads") used `refs #57` instead of `Closes #57`, so the tracker never
auto-closed even though the work is fully landed and evidenced.

Evidence of completion (all verifiable in-repo):
- **Commit**: `f63599c` on `main` — `TransferAdapter::probe_link` (ADR-011
  extended from #50's `probe_fleet`, default `Ok(None)`), the free-standing
  `prober::classify` pure fn shared by both legs, the S3 multipart prober
  (create → canary → K concurrent parts → ABORT — never complete), bridge
  delegation via `inner_spec`, journal contract unchanged.
- **RED pinned BEFORE implementation**: `scripts/issue57_red_evidence.md`
  (E0407 / E0425 ×10 / E0599 ×3), commit `b527683`.
- **Gates**: fmt + clippy `-D warnings` + **258/258** (247 + 8 `prober_s3`
  contracts incl. the 5 MiB floor interaction + 3 drill legs) — green on the
  committed tree, reproduced this session (recycle #7; toolchain restored
  from `scripts/install-toolchain.sh`, suite re-run 258/258).
- **Live drills ×2** (`IUM_S3_PROBE_DRILL=1`): per-conn 64 KiB/s →
  PerFlowCap fleet 4; shared 64 KiB/s bearer → BearerCap fleet 1; every leg
  leaves ListMultipartUploads EMPTY (the probe aborts its own upload).
- **STATUS.md**: the rung-2 row (2026-11-05) records the whole loop,
  including the two drill-discovered bugs fixed mid-loop (pid-derived port
  collisions; the unsound loopback `!PerFlowCap` assertion).

Lesson for the loop: the closing keyword is part of the deliverable — a
`refs #N` commit closes nothing. This repair closes the gap explicitly;
`git log --grep='Closes #57'` will stay empty by design, the evidence lives
here and in the STATUS row."""

req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/issues/57/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={
        "Authorization": f"Bearer {TOKEN}",
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
    },
    method="POST",
)
with urllib.request.urlopen(req) as r:
    comment = json.load(r)
    print("comment:", comment["html_url"])

req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/issues/57",
    data=json.dumps({"state": "closed",
                     "state_reason": "completed"}).encode(),
    headers={
        "Authorization": f"Bearer {TOKEN}",
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
    },
    method="PATCH",
)
with urllib.request.urlopen(req) as r:
    issue = json.load(r)
    print("issue #57 state:", issue["state"], "| reason:",
          issue["state_reason"])
