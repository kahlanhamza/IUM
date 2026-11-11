#!/usr/bin/env python3
"""Post the #109 live wire-facts rehearsal ledger (masked)."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/{path}",
        data=json.dumps(payload).encode() if payload else None,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


BODY = """## Live wire-facts rehearsal — PASS (2026-11-12, real API, masked logs)

Console-issued short-lived access token (owner-supplied, never logged).
Payload: 9,437,184 B, deterministic pattern, 3 chunks (4 MiB / 4 MiB / 1 MiB).
Script: `scripts/rehearsal109b.py`; raw transcript: `scripts/rehearsal109_ledger.txt`.

| # | Wire step | Live result |
|---|---|---|
| 1 | `POST /2/users/get_current_account` | HTTP 200 — probe path proven (this is what `probe` will call) |
| 2 | `POST /2/files/upload_session/start` (`close:false`) | HTTP 200 → `{"session_id": "pid_upload_session:…"}` |
| 3 | `append_v2` @ offset 0, 4 MiB body | HTTP 200 (empty body) |
| 4 | `append_v2` **BEHIND** offset (0 again, different 4 MiB body) | **HTTP 200 — SILENT NO-OP** (see finding A) |
| 5 | far-offset probe (100 MiB) | **HTTP 409 `incorrect_offset`**, `correct_offset: 4194304` — server truth exposed |
| 6 | `append_v2` realigned @ 4,194,304 | HTTP 200 |
| 7 | `append_v2` tail @ 8,388,608 (1 MiB) | HTTP 200 |
| 8 | `finish` (empty body, `CommitInfo{mode:overwrite, autorename:false}`) | HTTP 200 → `FileMetadata` **with `content_hash`** |
| 9 | **verify**: server `content_hash` vs local (Python `hashlib`, independent impl) | **MATCH** — `741069d32062eb7de12b3dc106b7bc2e7f3400901bd3d98780e4f4988a9f6737` |
| 10 | cleanup `POST /2/files/delete_v2` | HTTP 200 — no residue in the app folder |

### Finding A — behind-offset appends are SILENT NO-OPs (idempotent-retry tolerance)

Appending at an offset **behind** the server's cursor answers **200 and stores
NOTHING** (finding #4 + server truth #5: session still held exactly 4 MiB).
The documented `incorrect_offset` only fires for **AHEAD** offsets.

Adapter consequence (design-note addendum): `append_v2` is at-least-once safe
**iff the adapter always sends the same bytes for the same journaled offset**:

- crash *after* a wire-accepted append but *before* the journal advance →
  journal offset is BEHIND → the retry at that offset no-ops (200) → the
  journal advance by `len` is correct either way — the bytes are on the wire
  exactly once in effect;
- journal AHEAD of the server (should not happen; would mean journal
  corruption) → 409 → realign to `correct_offset`, journal the correction.

No extra round-trips needed for resume. This is the #10 409-realign pattern,
now live-confirmed with real wire shapes.

### Finding B — real error shapes (differ from the #108 doc reading)

- Status is **409** (not 400) for `incorrect_offset`.
- Error body shape: `{"error": {".tag": "incorrect_offset",
  "correct_offset": 4194304}, "error_summary": "incorrect_offset/", …}` —
  `correct_offset` is a **sibling of `.tag`**, not nested under a named key.
- Empty-body RPC POSTs must NOT carry `Content-Type: application/json` (a
  json content-type on an empty body tripped a server-side 500; bare POST is
  accepted — observed live, then reproduced clean).

### Acceptance status

- [x] Design note posted (contract mapping + token-loss UX)
- [x] Live wire-facts rehearsal with the owner-supplied app registration (masked logs) — **this comment**
- [ ] RED at HEAD posted on this issue — next, implementation follows
- [ ] `adapter_dropbox` lands behind the standard gates
- [ ] Verify refuses Done without the `content_hash` match (proven live at #9; pinned in the contract test)
- [ ] Full gates green + CI verdict chased
- [ ] Shell exposure rides the existing connect surface
"""

r = api("POST", "issues/109/comments", {"body": BODY})
print("ledger comment id:", r["id"])
