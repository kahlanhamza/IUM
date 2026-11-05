#!/usr/bin/env python3
"""File milestone M9 (new transports & release) + its issues, idempotently.

Owner directive 2026-11-05 ("Go ahead next phase"). The roadmap deferred the
next tranche to the owner's call twice (M6 verdict, M7 DoD); this milestone
answers it with: the Feishu Drive resumable lane (#56 — the owner's own
storage, wire facts rehearsed LIVE with the vault-held app credentials), the
S3-shaped prober follow-up on record since #48 (#57), and the 0.1.0 release
cut (#58). EXECUTED 2026-11-05. Re-run is a no-op: existing titles are
skipped and their numbers reported.

Repo token comes from .secrets/gh_token (never printed, never committed).
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path(".secrets/gh_token").read_text().strip()

MILESTONE_TITLE = "M9 — New transports & the 0.1.0 release"
MILESTONE_DESC = (
    "The roadmap's deferred tranche, answered: a third real transport (Feishu "
    "Drive resumable multipart — the owner's own storage, wire facts rehearsed "
    "live), the S3-shaped prober gap noted in #48 (measured profiles must cover "
    "both transports), and the v0.1.0 release cut (CHANGELOG release, versions, "
    "tag). Each rung through the full loop: wire facts -> design note -> RED -> "
    "implement -> drill -> gates -> one commit -> STATUS."
)

M56_BODY = """M9 rung 1. A third real transport: **Feishu Drive** (云空间) via the drive-v1
resumable multipart upload API — the owner's own storage, app credentials already
held in the secret vault. This is a genuinely resumable lane: prepare/part/finish
with a 24 h transaction window.

## Wire facts — rehearsed LIVE 2026-11-05 (all steps executed against the real API)

- **W0** `GET /drive/explorer/v2/root_folder/meta` → the app's own root folder token
  (`data.token`). Upload target for tenant-credential uploads (no user OAuth needed).
- **W1** `POST /drive/v1/files/upload_prepare` `{file_name, parent_type:"explorer",
  parent_node, size}` → `{upload_id, block_size, block_num}`. **block_size is
  platform-fixed at 4 MiB** (4194304 — observed on 4 KiB, 8 KiB, 9 MiB payloads);
  `block_num = ceil(size / 4 MiB)`. Transaction + progress valid **24 h** (docs).
- **W2** `POST /drive/v1/files/upload_part` — **multipart/form-data** (NOT JSON, NOT
  query): fields `upload_id`, `seq` (0-based), `size` (actual part bytes; tail block
  may be smaller), optional `checksum` (**Adler-32**, decimal string), and the binary
  part named **`file`**. Serial only: **no concurrency, 5 QPS, 10k/day** (docs;
  observed ~1 call/s fine).
- **W3** `POST /drive/v1/files/upload_finish` `{upload_id, block_num}` →
  `{file_token}`. Missing blocks → 1062010 "block missing" (finish is the
  completeness gate). Field is `block_num`, NOT `block_number`.
- **W4** Verify surface: `/drive/v1/metas/batch_query` (`request_docs[].doc_token`
  / `.doc_type`) returns `title/url/owner/…` but **NO size**; folder list
  (`GET /drive/v1/files?folder_token=…`) has **no size either**. Therefore
  **verify = streamed download + sha256** (see W10) — the strongest form of
  verify-before-declare anyway.
- **W5** Cleanup: `DELETE /drive/v1/files/{file_token}?type=file` → 200.
- **W6** Multi-block: 9 MiB → `block_num=3` (4+4+1 MiB); tail block uploads with its
  actual size.
- **W7** **Out-of-order parts ACCEPTED**: seq=1 uploaded before seq=0 → 200. The
  chunk pool's natural completion order can drive parts without serialization
  (subject to the 5 QPS / no-concurrency doc constraint → lane is single-worker
  like tus, `max_parallel=1`).
- **W8** Error wire: wrong field names → 99992402 field validation (prepare/finish,
  JSON) or 1061002 params error (part, multipart); wrong binary field name
  (`data` instead of `file`) → 1061002.
- **W10** `GET /drive/v1/files/{file_token}/download` **streams the file content
  directly**; downloaded bytes hash byte-exact to the source (sha256 match, live).

## Design sketch (to be challenged by the design note)

- New adapter `adapter_feishu::FeishuAdapter` implementing `TransferAdapter` over
  the drive-v1 files API; credentials via env (`IUM_FEISHU_APP_ID` / `IUM_FEISHU_APP_SECRET`)
  — **env-gated drills** (the S3 kill-9 pattern); no secrets in the repo.
- Chunk geometry: the planner composes with the adapter-declared cap (ADR-011);
  this lane prescribes **4 MiB blocks** — chunks map 1:1 onto Feishu blocks so a
  journal-confirmed chunk == an uploaded block (resume re-derives from the journal;
  upload_id lives in the session record; 24 h expiry → 1061021 → re-prepare arm).
- Verify trait: streamed download + sha256 vs source.
- Single worker per job (`max_parallel=1`) — serial part uploads per the docs.

## RED plan

Compile-fail/contract tests for the new adapter's chunk↔block mapping, the
Adler-32 checksum framing, the finish-completeness gate, and the verify surface
— failing first against the stub, then the live drill (`tus-test.sh`-style
env-gated script) proving: 9 MiB 3-block upload byte-exact (download+sha256),
out-of-order part completion, kill-9 mid-lane resume from the journal.
"""

M57_BODY = """M9 rung 2. The follow-up on record since #48: the link prober measures the tus
lane only — the S3 multipart lane gets its own shaped probe so the measured-profile
architecture (`link_profile` journaled once, planner obeys `replan_measured`,
fleet seeded from the measurement) covers BOTH real transports.

## Scope

- `probe_link` grows an S3-shaped canary leg: small multipart transactions against
  the adapter's S3 lane (the pinned SeaweedFS/rclone servers, plus the real-cred
  lane when the owner supplies one), measuring per-flow vs aggregate the same way
  the tus prober does.
- Same journal contract: one `link_profile` event at the probe boundary, no
  fabrication on failure (`link_profile_skipped`), planner obeys the measured
  piece.
- Wire facts first: what does a multipart prober need (part size floor, create/
  complete multipart round-trips, abort cost) — rehearsed live before the design
  note claims anything.

## RED plan

Contract tests for the S3 probe classification (per-flow cap vs bearer cap vs fat
pipe) pinned with the same pure-fn shape as the tus prober tests; env-gated live
drill against the pinned S3 servers; a failed probe never loses a job.
"""

M58_BODY = """M9 rung 3. Cut **v0.1.0** — the semver policy (docs/05) has governed the repo
since M5; time to execute it end to end.

## Scope

- CHANGELOG.md: promote `[Unreleased]` → `[0.1.0] — 2026-11-XX` with issue-cited
  entries for M5→M9 (the M5 rule: an entry without verifiable issue numbers does
  not belong).
- `ium-core` version → 0.1.0; `ium-app` version aligned.
- Git tag `v0.1.0` on the release commit; GitHub Release with the release notes
  (honest scope: what the engine guarantees today, measured).
- CI green on the tag; STATUS row.

## Gate

The release commit is cut ONLY after #56 and #57 are closed and the suite is
green on the committed tree (gate.sh) — the tag is a promise, not a hope.
"""

ISSUES = [
    ("M9#1 Feishu Drive lane: the resumable multipart upload API as a first-class transfer adapter (wire facts rehearsed live)", M56_BODY),
    ("M9#2 The prober covers the S3 lane: measured profiles for multipart uploads (the #48 follow-up on record)", M57_BODY),
    ("M9#3 v0.1.0: cut the release — CHANGELOG release section, versions, tag, release notes", M58_BODY),
]


def api(method, path, payload=None):
    url = f"https://api.github.com/repos/{REPO}/{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        url, data=data, method=method,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-file-m9",
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
    mnum = next((m["number"] for m in milestones if m["title"].startswith("M9 —")), None)
    if mnum is None:
        status, m = api("POST", "milestones", {"title": MILESTONE_TITLE, "description": MILESTONE_DESC})
        mnum = m["number"] if status == 201 else None
        print(f"milestone created: #{mnum}" if mnum else f"milestone FAILED: {status} {m}")
    else:
        print(f"milestone exists: #{mnum}")

    status, existing = api("GET", "issues?state=all&per_page=100")
    if status != 200:
        print(f"issue list failed: {status} {existing}")
        return 1
    titles = {i["title"]: i["number"] for i in existing if isinstance(existing, list)}
    for title, body in ISSUES:
        got = titles.get(title)
        if got:
            print(f"issue exists: #{got} — {title[:60]}")
            continue
        status, issue = api("POST", "issues", {
            "title": title, "body": body,
            "milestone": mnum, "labels": ["enhancement"],
        })
        if status == 201:
            print(f"issue created: #{issue['number']} — {title[:60]}")
        else:
            print(f"issue FAILED: {status} {str(issue)[:300]} — {title[:60]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
