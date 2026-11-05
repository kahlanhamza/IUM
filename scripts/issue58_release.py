#!/usr/bin/env python3
"""Create the GitHub Release for tag v0.1.0 — honest scope, measured facts
only (all numbers from STATUS.md rows / issue evidence). Token never printed."""
import json
import os
import urllib.request

REPO = "kahlanhamza/IUM"
TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {"Authorization": f"Bearer {TOKEN}",
       "Accept": "application/vnd.github+json",
       "Content-Type": "application/json"}

NOTES = """# IUM v0.1.0 — the first release cut

**IUM is an upload engine** (`ium-core`, Rust) that applies download-manager
reliability engineering to uploads: a SQLite-WAL journal is the only truth,
every transition is journaled before it executes, and nothing is declared
done that has not been verified on the wire.

This tag closes milestones M1→M9 and lands the first M10 rung. Every claim
below is measured — evidence lives in the issue tracker, not in prose.

## Guarantees (measured, drilled)

- **Verify-before-declare** — `verify` is a REQUIRED adapter trait method
  (#1, ADR-010): ListParts reconcile + HEAD before Complete; a resumed-but-
  refused verify triggers repair, never a declaration (#24).
- **Zero-re-upload resume** — SIGKILL mid-body on every lane, byte-level
  resume from the journaled token; the kill-9 drill proves the re-sent byte
  count EXACTLY zero (#9, #11, #30 — the 64 KiB flush-race ceiling was
  root-caused and made exact).
- **Counted retries, honest failure** — per-error-class policy, backoff,
  `Retry-After` honoring, deterministic exhaustion → Failed; every failure
  path terminates in Done or Failed.
- **Cancel is real** — deterministic mid-flight cancel, server state cleaned
  (HEAD 404), pause latency measured in milliseconds (#29, #26).
- **Slow links converge** — the 600 s wall is gone (#42): idle-progress
  timeout only; tus legally pieces to 64 KiB (#44); the 3/8/50/160 KB/s CI
  ladder rides 99%+ of wire at every rung (#45); a 512 B/s unmeasurable link
  still resplits via the frozen rule (#28).

## Lanes (all journaled, all resumable, all verified)

- **HTTP one-shot** (reqwest+rustls, streaming, HEAD verify).
- **TUS 1.0.0** against pinned tusd 2.10.1 — byte-level resume, 409 realign
  that absorbs drifted-but-server-acked bytes (#43), concatenation lane (#50).
- **S3 multipart** against pinned SeaweedFS 4.51 (+ gofakes3) — SigV4
  hand-rolled, pinned to AWS vectors (#6); the probe ABORTS its own upload,
  never completes it (the 5 MiB floor is server-dependent — #57).
- **The rclone bridge (#63)** — ONE adapter = **74 backends day one**
  (Drive, OneDrive, Dropbox, MEGA, Box, pCloud, Koofr, the S3 family…):
  `adapter_rclone` spawns/attaches `rclone serve s3` and delegates every
  transfer to the proven S3 adapter — resume across bridge restarts from the
  journal alone, drilled live (kill the engine, fresh adapter, missing part
  re-uploaded, byte-exact).
- **Feishu Drive (#56)** — resumable multipart as a first-class lane
  (platform-fixed 4 MiB blocks, out-of-order parts, 24 h session honesty).
- **Pseudo-lanes**: RaptorQ FEC parity (any K of K+R rebuilds byte-exact,
  wire 1.2503× — #52) and entropy-gated zstd (86.6% wire saved on
  compressible payloads, declined honestly in 0.03 s on video-like bytes —
  #53); bonded multi-path uploads (1.71× single-bearer in the live drill —
  #51).

## The speed architecture

The engine MEASURES instead of assuming (#41, #48, #57): the link profile is
probed once, journaled, and the planner OBEYS it; the fleet waterline is
seeded from the journal and adapted by a live AIMD controller both directions
(#49); adaptive resplit fires from measured coarseness (#27) and on
unmeasurable starvation (#28); eviction is strike-counted, never blind (#25).

## The shell

Tauri 2 + React (unfrozen #14): jobs table with live progress + honest
badges, per-chunk inspector, engine-gated actions (#15, #16); the UI's brain
is pure functions under vitest; a real-shell WebDriver e2e drives the
packaged app on 3 OS (#35, #37).

## Honest limits (filed, not done)

- #59 native Google Drive OAuth depth, #60 YouTube, #61 native MEGA, #64 the
  TeraBox tranche (fork lane + census) — BLOCKED-ON owner credentials.
- #62 socials publishing (TikTok/IG/FB) — research spike, app-review reality,
  no code by design.
- Feishu is VAULT-ONLY going forward (owner directive): tokens and research
  notes, not an integration target.

## Provenance

`cargo test -p ium-core` → **258 passed / 0 failed** on this tagged tree;
fmt + clippy `-D warnings` clean; `scripts/gate.sh` GREEN on the committed
tree (detached worktree, ADR-007). CI: 3-OS matrix (ubuntu/macos/windows)
for engine + shell + e2e + nightly drills. The CHANGELOG carries **53
distinct issue-cited entries** — the whole M1→M10 record, audited at cut
time (#58: 16 missing entries were found and written from the STATUS rows,
not from memory).

## Build

```bash
cargo build -p ium-core          # the engine, no Tauri deps needed
cargo test -p ium-core           # 258 tests; env-gated live drills skip loud
# the desktop shell:
cd ium-app && npm install && npm run tauri dev
```

License: MIT.
"""

req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/releases",
    data=json.dumps({
        "tag_name": "v0.1.0",
        "name": "IUM v0.1.0 — the first release cut",
        "body": NOTES,
        "draft": False,
        "prerelease": False,
        "make_latest": "true",
    }).encode(),
    headers=HDR,
    method="POST",
)
with urllib.request.urlopen(req) as r:
    rel = json.load(r)
    print("release:", rel["html_url"])
    print("tag:", rel["tag_name"], "| id:", rel["id"])
