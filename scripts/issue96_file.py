#!/usr/bin/env python3
"""File issue #96: the upgrade-path bar — the N-1 journal pin (Lane A of the release-prep order)."""
import json, urllib.request, pathlib

TOKEN = pathlib.Path(".secrets/gh_token").read_text().strip()

title = "The upgrade-path bar: a v(N-1) mid-flight journal must resume byte-exact at N — the fixture pin"

body = """## Why this issue exists

The release-prep order (owner, 2026-11-10) named the gap: IUM has never run the
big-tech test class **N-1 → N upgrade**. We install fresh; migrations are
unit-tested reopen-safe; but nobody has ever opened a journal *written by the
previous release* with the *current* code and resumed the job. The next
release (v0.2.1) ships real engine fixes; the release after it will eventually
append `MIGRATIONS` v2 — the day that happens, this lane is the only thing
standing between us and a bricked resume.

## Wire facts (checked 2026-11-10, HEAD 9902779 vs tag v0.2.0)

- **Schema**: both ends are journal schema **v1 only** — `MIGRATIONS` has one
  entry, `PRAGMA user_version` = 1 at HEAD *and* at v0.2.0. No schema
  migration has ever been appended. The drill therefore pins *forward-compat
  of the rows + event stream*, and stands guard for the future v2.
- **Events evolved anyway**: v0.2.0 never wrote `link_profile`,
  `link_profile_skipped`, `leases_reclaimed`, `drive_pool_snapshot` or the
  `chunk_error` reason-chain payloads (#95 arc). The opener must tolerate a
  journal that LACKS today's event kinds (it does: state is read from the
  rows, never from caller arguments — ADR-002) — and the drill must prove it.
- **Settings**: shell persistence is localStorage (UI concern,
  `SettingsDialog.tsx`), not journal truth — out of scope for the engine pin,
  noted for the release notes.

## Design note

1. **The fixture is the N-1 artifact.** A journal produced by the v0.2.0 tag
   code itself (git worktree at `v0.2.0`, v0.2.0's own `SessionJournal` API:
   `insert_job` → `replace_chunks` → `set_job_state_event(Running)` →
   `update_chunk` confirms → `checkpoint_wal`), captured mid-flight: one job
   Running, several chunks Confirmed/Uploaded, one Leased, the rest Pending —
   the exact shape the longhaul pause/replay leg leaves behind. Committed as
   `ium-core/tests/fixtures/upgrade_v020_journal.db` with its sha256 in the
   test header. Regeneration is scripted (`scripts/upgrade_fixture.sh`), the
   same contract as the video fixtures.
2. **The pin (happy path)**: at HEAD, open the fixture → the manager recovers
   → drive the remaining bytes through the loopback recorder → **Done**,
   read-back sha256 exact, and the recorder saw **zero re-upload of the
   confirmed ranges** (the upgrade must not re-send what v0.2.0 already
   confirmed). `user_version` stays 1; no spurious migration.
3. **The detector-canary (hostile fixtures)**: mutate copies — an unknown
   event kind, an unknown chunk state, a job row with a state string from a
   hypothetical future — the opener must respond honestly (skip loud /
   JournalError), never panic, never silently claim progress it cannot see.
   Proves the pin can detect what it claims to detect.
4. **The future-v2 guard**: the test asserts `MIGRATIONS` is append-only by
   pinning the current count; a changed count without a new fixture fails the
   lane loud.

## Acceptance

- [ ] Fixture committed from v0.2.0 code + regeneration script + sha256.
- [ ] Happy-path upgrade test: resume → Done byte-exact, zero re-upload.
- [ ] Hostile-fixture tests: unknown kinds/states handled honestly, no panic.
- [ ] `user_version` pin + MIGRATIONS append-only guard.
- [ ] Gates green; CI green; CHANGELOG [Unreleased] Started.

*Self-note: this issue closes only with the fixture's provenance receipts in
the closing comment (worktree commit hash, sha256, generation log).*
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": title, "body": body, "labels": []}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
             "Content-Type": "application/json"},
    method="POST")
with urllib.request.urlopen(req) as r:
    d = json.load(r)
    print(f"filed #{d['number']}: {d['title']}")
    print(d["html_url"])
