#!/usr/bin/env python3
"""Post the lane-closure evidence comment on issue #103 (the destination lane).

Evidence-first closure: every acceptance checkbox ticked with the artifact
that proves it — commits, test counts, the CI run, and the local battery.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from file_issue import api

BODY = """## The lane is landed and green — both batteries

**Local gate** (committed tree `a388fbd`, `scripts/gate.sh` in a detached
worktree — what you push is what you tested): **535 tests, 0 failures** —
- fmt CLEAN, clippy `-D warnings` CLEAN, `cargo doc -D warnings` CLEAN
- core: **346 tests across 51 lanes** — including the new `providers_registry`
  (7: every preset's sample URL through the REAL `parse_target`), `sigv4_presign`
  (4: two published-anchored vectors, determinism, input-sensitivity, expiry
  bounds), `s3_check_credentials` (6: loopback mock S3 — happy / 403 / 404 /
  transport-refused / malformed / bucket-only), the vault lane, the trybuild gate
- frontend: `tsc strict` + `vite build` + **vitest 189/189** (providers 11,
  CreateJobForm 5, share 4, InspectorDrawer 3 among them)

**CI — the billing block LIFTED and the first live verdict since the private
flip is green**: run **43008204325** on `a388fbd77b64` — **8/8 jobs success**
(`fmt` 6/6; `core` ×3 OS 18/18 steps each; `ium-app` ×3 OS 14/14 each;
`app-e2e` 16/16 — the WebDriver lane passed against the new provider picker,
which is the e2e-seams-byte-compatible claim proven on the real wire).
<https://github.com/kahlanhamza/IUM/actions/runs/43008204325>

## Acceptance

- [x] providers.json + loader + schema tests (RED first) — ONE JSON
      (`ium-app/src/data/providers.json`, 7 presets), consumed by the TS
      loader AND the engine-side drift guard; RED captured pre-implementation
- [x] create-form picker + keys guidance, custom mode byte-identical —
      7 presets with key-console deep links + recipes; Custom URL the default
- [x] S3 `check_credentials` + mock-S3 wire tests + settings `endpoint` field
      — zero-byte Create + best-effort Abort on an `ium-probe/` key; the note
      discloses exactly what the probe touches
- [x] `probe_credentials` s3 arm — drafts in memory, no session orphaned
- [x] `sigv4::presign` + vectors — minted by the independent Python reference
      (`scripts/presign_reference.py`), which now **self-checks published AWS
      Vector B on every run and refuses to mint on drift** (`ed766f5`); vectors
      pinned on AWS's public documentation example key
- [x] `share_link` command + Inspector copy-link moment — journaled endpoint
      through the engine's parser, vault/env keys, 24h TTL, nothing
      auto-publishes
- [x] USERGUIDE §5 rewrite, README providers list, ADR-013, CHANGELOG
- [x] full battery GREEN locally; CI chase recorded — chase RESULTED GREEN

## Honesty notes on the path here

1. The previous STATUS row's "full local battery green" outran its evidence:
   the stored battery log ends in ENOSPC (`RC=101`, trybuild disk-full). This
   session re-drove the battery from zero; the record now reflects the fresh run.
2. The first re-drive hit `ld` SIGBUS — the sandbox's 9.9G disk filled by a
   6G full-debuginfo build. Fixed structurally: `line-tables-only` debuginfo
   (debug-assertions stay ON — CI semantics preserved), codified in the
   self-healing toolchain config (`415e1ba`, `a388fbd`).
3. The presign anchoring claim was not demonstrable as committed (a placeholder
   key literal in the new files); made runnable and re-minted (`ed766f5`).

Commits: `3f9d982` (engine half) · `e26a7cb` (app half) · `ed766f5` (anchor)
· `415e1ba`/`a388fbd` (cache slimming) — all on `origin/main`.
"""

status, res = api("POST", "issues/103/comments", {"body": BODY})
print(f"comment: http-{status} id={res.get('id')}")
sys.exit(0 if status == 201 else 1)
