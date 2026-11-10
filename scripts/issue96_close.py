#!/usr/bin/env python3
"""Post the #96 closure evidence: provenance receipts for the fixture + the pin results."""
import json, urllib.request, pathlib

TOKEN = pathlib.Path(".secrets/gh_token").read_text().strip()

body = """## CLOSURE — the N-1 → N pin is landed, the detector is proven, receipts below

### Fixture provenance (the N-1 artifact)

- **Generator commit**: `60c5043` carries `scripts/upgrade_fixture.sh` + `scripts/upgrade_fixture/gen_upgrade_fixture.rs`.
- **Provenance**: a `git worktree` at tag **v0.2.0**; the generator copied in; the REAL v0.2.0 `JobManager`
  (`create_job` → planner → leases → confirms through v0.2.0's own journal code) drove a 12 MiB job;
  the drive future was dropped **mid-call** — the in-process twin of a SIGKILL'd v0.2.0; WAL folded.
- **Fixture sha256**: `d393fb592bf9dd81befdf88e03b1e60bc6474fad4229ce0af1665a89a48fddec`
  (`ium-core/tests/fixtures/upgrade_v020_journal.db`, committed).
- **What v0.2.0 left on disk** (generator's own print, pinned by the test):
  job `01M71E5TRWNCFZC9RJN54WPRA7` Running · 12,582,912 B · chunks 0–2 Uploaded full ·
  chunk 3 **Leased at the half-confirmed frontier (1,572,864 B)** ·
  event kinds `job_created, state, probe, replan_cap, chunk_confirmed ×4` — v0.2.0-era only.

### The pins (green at HEAD `60c5043`, committed-tree gate GREEN)

1. `user_version` is 1 before and after HEAD's `migrate()` — **the MIGRATIONS tripwire**: a v2
   landing without a new N-1 fixture fails this lane loud, with the instruction in the message.
2. Rows round-trip exactly (job fields, all four chunk rows, event kinds).
3. **Zero re-upload asserted on the wire**: the resume made exactly ONE adapter call —
   idx 3, `resume_from == 11_010_048` (the frontier) — chunks 0–2 and the confirmed half
   never re-sent; total moved = 12 MiB − 11,010,048 = 1,572,864 B exactly.
4. **Byte-exact**: the received tail equals the deterministic source (same LCG as the generator).
5. **Future-kind tolerance**: a `kind_from_the_future` event appended to a copy — reader
   tolerates it end-to-end, resume still Done (what release N+2 will leave behind when
   this binary is the N-1).

### The detector proven (acceptance 2/3/4 in one)

- The future-kind test proves the reader's tolerance contract by INJECTION.
- The tripwire is the literal `user_version` read from the real fixture (1) — it trips the
  moment `MIGRATIONS` grows.
- Generator API drift was caught and fixed in-session (`ChunkState` import path, `JobRecord`
  field set at v0.2.0) — the script is idempotent: rerun regenerates identical bytes.

Local gates: fmt clean, clippy `-D warnings` clean, `cargo test -p ium-core` 47/47 binaries
green incl. the two new tests. CI verdict to follow on this push.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/96/comments",
    data=json.dumps({"body": body}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
             "Content-Type": "application/json"},
    method="POST")
with urllib.request.urlopen(req) as r:
    d = json.load(r)
    print("posted:", d["html_url"])
