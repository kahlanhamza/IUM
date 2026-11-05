#!/usr/bin/env python3
"""File the M10 tranche (owner's big list, chat 2026-11-05) as 4 GitHub issues.

Milestone 9 = "M10 — The big list: Drive, YouTube, MEGA, and the socials research".
Every issue: wire-facts-first discipline stated, adapter-contract fit, verify
semantics, and the BLOCKED-ON credential dependency called out up front.
"""
import json
import sys
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
MILESTONE = 9
REPO = "kahlanhamza/IUM"


def api(method, path, body=None):
    req = urllib.request.Request("https://api.github.com" + path, method=method)
    req.add_header("Authorization", "Bearer " + TOKEN)
    req.add_header("Accept", "application/vnd.github+json")
    data = json.dumps(body).encode() if body is not None else None
    with urllib.request.urlopen(req, data=data, timeout=30) as r:
        return json.loads(r.read().decode())


ISSUES = [
    {
        "title": "M10#1 The Google Drive lane: resumable upload over files.create (uploadType=resumable)",
        "labels": ["enhancement", "engine", "reliability"],
        "body": """## Why
The owner's platform list (chat 2026-11-05) names the famous-name platforms. Google Drive is the canonical storage target with a first-class **resumable upload API** — the most direct fit with the IUM adapter contract (probe / upload_chunk / finalize / verify / reattach).

## The wire (to be rehearsed LIVE before any design note — the loop's rule; wire facts are rehearsed, never read from docs)
- `POST https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable` (JSON metadata body) → `200` + `Location` header = the session URI.
- `PUT <session-uri>` per chunk (256 KiB multiples; final chunk carries the exact `Content-Length`) → `200/201` + the file resource.
- Resume probe: `PUT <session-uri>` with `Content-Range: bytes */N` → `308` + `Range:` header = the next offset — maps 1:1 onto `reattach()`.
- Verify: `GET /drive/v3/files/{id}?alt=media` streamed + sha256; Drive ALSO exposes `md5Checksum` server-side — cross-check both (stronger than Feishu's sizeless wire, issue #56's lesson).
- Auth: OAuth 2.0 refresh token → access token (~1 h expiry) → the existing session/expiry taxonomy (the #56 `ExpiredSession` verdict shape).

## Acceptance (same bar as #56)
Live wire-facts W-set pinned on this issue → design note → RED → `adapter_gdrive` over the SAME trait → journaled probe event → live drill byte-exact (upload → kill → fresh-adapter resume → verify) → full gates.

## BLOCKED ON (owner)
- Google Cloud project + OAuth consent screen + OAuth client (Desktop app).
- IUM needs `client_id` + `client_secret` (goes to `.secrets/`, 600, never committed) and a refresh token via the consent flow.
- Scope: `https://www.googleapis.com/auth/drive.file` (per-file, least privilege).

No code before the credentials exist.
""",
    },
    {
        "title": "M10#2 The YouTube lane: resumable videos.insert — publishing semantics on the owner's channel",
        "labels": ["enhancement", "engine"],
        "body": """## Why
Same Google OAuth family as the Drive lane (M10#1) — shares the client + consent groundwork, adds scope `https://www.googleapis.com/auth/youtube.upload`. The owner's list names YouTube explicitly.

## The wire (rehearse LIVE first, same rule)
- `POST https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status` (video metadata JSON) → `200` + `Location` session URI.
- `PUT <session-uri>` chunks; `308` + `Range` on resume probe — the same shape as Drive.
- verify-before-declare needs an HONEST VARIANT here: the destination is a **video on a channel**, not a byte file. Server-side processing states (uploaded → processing → processed) mean byte-level verify is only possible via a download of the processed rendition. The verify semantics (API status poll vs byte check) get pinned in the design note BEFORE implementation — this is the issue's core design risk.

## Acceptance
Same loop bar as #56/#59; the status-poll verify variant journaled; live drill = small test video end-to-end on the owner's channel (unlisted for drills).

## BLOCKED ON (owner)
- The M10#1 OAuth client + **YouTube Data API v3 enabled** on the same project.
- Unverified apps carry a 6-videos/day upload cap — ample for drills; noted, not a blocker.

No code before M10#1's credentials exist (this lane rides them).
""",
    },
    {
        "title": "M10#3 The MEGA lane: the crypto-heavy API — client-side AES + ECDH key hierarchy",
        "labels": ["enhancement", "engine", "reliability"],
        "body": """## Why
The owner's list says "Megadrive" — read as **MEGA (mega.nz)**, the zero-knowledge storage platform (correct the issue if a different service was meant). It is a real storage target, but the heaviest lane on the list.

## What makes it heavy (honesty up front)
- Zero-knowledge design: files are encrypted **client-side**; the API never sees plaintext. Upload flow = pre-login key derivation → session + master key → per-file AES key + attribute blob → upload-URL request → chunked cipher upload → completion node.
- Crypto must come from audited crates (e.g. `aes`, `cbc`, `rsa`, `ecdh` via `ring`/`RustCrypto`) — never hand-rolled primitives.
- Chunk boundaries on MEGA's wire are platform-fixed (their own size ladder) → the #56 prescribed-geometry channel (`uniform_block` + `plan_uniform_tail`) is the likely fit, proven pattern.

## Verify semantics (actually strong)
MAC over the ciphertext + download → decrypt → sha256 — a stronger end-to-end check than most lanes.

## Acceptance
Same loop bar; the wire-facts rehearsal will be substantial (key hierarchy vectors pinned before any adapter code).

## BLOCKED ON (owner)
- MEGA account email + password (or an exported session) — held in `.secrets/` only, never printed, never committed.
""",
    },
    {
        "title": "M10#4 The socials spike: TikTok / Instagram / Facebook publishing APIs — wire-facts reality check (no code)",
        "labels": ["enhancement", "audit", "process"],
        "body": """## Why
The owner's list includes TikTok, Instagram, Facebook. Honest framing first: these are **publishing platforms, not storage** — the destination is a post/reel on an account, and the gates to reach the wire are organizational as much as technical.

## What the spike pins (research output only — NO adapter code from this issue)
Per platform, a wire-facts note:
- **TikTok** — Content Posting API: draft upload (`POST /v2/post/publish/video/init/`), then publish; OAuth per account; app review required for unaudited `video.publish` scopes.
- **Instagram** — Content Publishing API: reels require a **Business/Creator account** linked to a Facebook Page + app review; container-then-publish two-step; no byte-level verify (processing).
- **Facebook** — Video API (page/user); `publish_video` permission needs app review; resumable upload sessions exist (similar shape to Drive's 308 pattern); verify = status poll only.

## The go/no-go criteria (spike deliverable)
1. Can the owner actually get the app approved (business verification, review timelines)?
2. Verify semantics: status-poll-only is acceptable to the owner or not (the verify-before-declare invariant, issue #1).
3. Content policy vs the engine's nature (an uploader that retries/resumes may trip abuse heuristics).

If GO on any platform → separate adapter issues with credentials as the blocker.

## BLOCKED ON (owner)
Developer apps on each platform (TikTok for Developers, Meta for Developers) + the app-review decision. App review is the long pole — start it now if the socials matter; the spike proper waits for credentials.
""",
    },
]

for spec in ISSUES:
    created = api("POST", "/repos/%s/issues" % REPO, {
        "title": spec["title"],
        "body": spec["body"],
        "labels": spec["labels"],
        "milestone": MILESTONE,
    })
    print("#%d filed: %s" % (created["number"], created["title"]))
