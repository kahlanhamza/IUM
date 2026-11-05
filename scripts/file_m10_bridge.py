#!/usr/bin/env python3
"""File the rclone-bridge pivot (owner directive chat 2026-11-05):
- #63 the bridge lane (breadth: 74 backends over the EXISTING adapter_s3)
- #64 the TeraBox tranche (fork lane + native API research spike)
- comment on #56: Feishu is VAULT-ONLY (owner clarification)
- update the M10 milestone description with the new order
"""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
REPO = "kahlanhamza/IUM"
MILESTONE = 9  # GitHub milestone 9 = "M10 — The big list"


def api(method, path, body=None):
    req = urllib.request.Request("https://api.github.com" + path, method=method)
    req.add_header("Authorization", "Bearer " + TOKEN)
    req.add_header("Accept", "application/vnd.github+json")
    data = json.dumps(body).encode() if body is not None else None
    with urllib.request.urlopen(req, data=data, timeout=30) as r:
        return json.loads(r.read().decode())


ISSUE_63 = {
    "title": "M10#1 The rclone bridge lane: 74 backends over the EXISTING S3 adapter (serve s3 + sigv4)",
    "labels": ["enhancement", "engine", "reliability"],
    "body": """## Why
Owner directive (chat 2026-11-05): focus on **the famous platforms and cloud storages most people and companies use — "there is a lot of them"**, with rclone as the carrier the owner half-remembered ("an adapter or something called cclone"). Pinned: **rclone v1.75.2 ships 74 backends** (Google Drive, OneDrive, Dropbox, Mega, Box, pCloud, Koofr, every S3 flavor, WebDAV/FTP/SFTP servers...), and `rclone serve s3` exposes ANY of them over the S3 wire that IUM's **EXISTING** `adapter_s3` + `sigv4` module already speaks. One lane = breadth day one. The native adapters (#59-#61) remain the depth upgrades with full IUM-native guarantees.

## Wire facts — rehearsed LIVE 2026-11-05 (`scripts/wirefacts63/`, rclone v1.75.2, local fs root)
- **W1**: `rclone help backends` = 74. Famous names present: `drive`, `onedrive`, `dropbox`, `mega`, `box`, `pcloud`, `koofr` (+ all S3 flavors). NOT present: `terabox` (fork territory — #64), icloud (absent in this build).
- **W2**: single PUT (3 MiB) → `200`; object lands under the served root's bucket dir.
- **W3**: GET back → **byte-exact** (`cmp` clean).
- **W4**: multipart init `POST /bkt/key?uploads` → real `UploadId` (XML, proper S3 shape).
- **W5**: part PUTs → `200` + REAL per-part ETags (md5 with a local root; `--etag-hash` chooses).
- **W6**: complete (real ETags) → `CompleteMultipartUploadResult`, ETag `...-2` (multipart suffix).
- **W7**: **9 MiB via 2-part multipart → GET back byte-exact.** Server stages parts at `bkt/.rclone_temp_multipart_<id>` and rcat-materializes on complete — the same shape as IUM's #50 concatenation lane.
- **W8 (quirk, LOUD)**: `--auth-key ak,sk` AND the CSV-quoted form BOTH → `403 InvalidAccessKeyId` in v1.75.2 (key never registers server-side; `INFO serve s3: Access Denied`). Anonymous mode works (`No auth provided so allowing anonymous access`). Bridge plan: bind `127.0.0.1` only — the bridge is a local relay; real auth is the backend remote's concern (rclone token in `RCLONE_CONFIG_*` env), not the S3 hop's.
- **W9**: `librclone` (official C library) + the `librclone` Rust crate (docs.rs, auto-compiles rclone) = the recorded in-process FFI alternative if a serve process is ever unwanted.

## Design sketch (design note BEFORE code — the loop)
- A bridge lane over the EXISTING `adapter_s3`: a supervisor spawns `rclone serve s3 --addr 127.0.0.1:<port> <remote:path>` with `RCLONE_CONFIG_*` env vars (no rclone.conf on disk), waits for the listen line, runs IUM's existing S3 probe, then rides the normal contract (probe / upload_chunk / finalize / verify / reattach).
- Resume honesty: the S3 multipart session lives in the SERVE process. IUM-crash → re-attach if the serve process is alive (ListParts on the journaled UploadId); serve-crash → fresh session, the chunk pool re-drives (journal is the truth — ADR-002 holds). The serve process becomes part of the lane's session state, journaled like every other lane fact.
- Verify = streamed GET-back through the bridge + sha256 (the #56 pattern; ETag cross-check where the backend exposes a hash).
- **The drill needs NO user credentials**: `rclone serve s3 :local:` or `:memory:` rehearses the entire lane locally; real backends plug in via env only.

## Acceptance (same loop bar)
Design note → RED → implementation → journaled probe event (`bridge_plan`: remote, port, serve pid, UploadId) → live drill (kill IUM / kill serve → resume → byte-exact) → full gates.
""",
}

ISSUE_64 = {
    "title": "M10#5 The TeraBox tranche: the fork lane (bclone / rclone-extra) + native API research — wire-facts spike",
    "labels": ["enhancement", "audit", "process"],
    "body": """## Why
The owner's list (chat 2026-11-05) names **TeraBox "kind of services — there is a lot of them"**: consumer clouds like TeraBox, MediaFire, Degoo, JioCloud, Baidu Netdisk, Aliyun Drive, Quark... Live fact: official rclone v1.75.2 has **NO terabox** among its 74 backends (probed locally today). The community fills the gap with forks — and the #63 bridge doesn't care WHICH rclone serves the S3 wire.

## Facts pinned (2026-11-05, live + web)
- Official rclone v1.75.2 `rclone help backends` = 74, no terabox (LIVE probe, `scripts/wirefacts63/`).
- Forks adding TeraBox: **bclone** (github.com/BenjiThatFoxGuy/bclone — Alist, Alldebrid, iCloud Photos, Teldrive, Terabox, Tmpfs) and **rclone-extra** (forum.rclone.org/t/53142, Dec 2025 — Terabox, Teldrive, Alldebrid, Alist). Upstream tracker: rclone issue #7496.
- A fork build of `rclone serve s3` exposes TeraBox over the SAME bridge lane as #63 — **zero IUM changes** if the fork's serve s3 matches (to be probed: the fork may lag upstream's serve s3).

## The spike decides (research output only — NO adapter code from this issue)
1. **Fork lane**: build rclone-extra/bclone, probe their `serve s3` (multipart parity vs upstream W4-W7), rehearse a terabox remote LIVE with owner credentials → the bridge carries it. Risks to pin: unofficial reverse-engineered API (TOS exposure), fork maintenance lag, cookie/token auth fragility.
2. **Native lane**: is a stable native TeraBox adapter realistic (community API knowledge: dillondrummond/terabox-uploader et al.)?
3. **The "kind of services" census**: which consumer clouds actually expose usable upload APIs vs web-only walls — the honest go/no-go list the owner deserves.

## BLOCKED ON (owner)
A TeraBox account (login or API cookies) for live wire-facts; fork binary builds.
""",
}

for spec in (ISSUE_63, ISSUE_64):
    created = api("POST", "/repos/%s/issues" % REPO, {
        "title": spec["title"],
        "body": spec["body"],
        "labels": spec["labels"],
        "milestone": MILESTONE,
    })
    print("#%d filed: %s" % (created["number"], created["title"]))

# Comment on #56: the owner's vault-only clarification
comment = api("POST", "/repos/%s/issues/56/comments" % REPO, {
    "body": """**Owner clarification (chat 2026-11-05):** the Feishu credentials were provided for the **secret vault** (tokens / research storage across sandbox recycles) — *not* as an integration target. The adapter lane stays closed as built (it works, it is the owner's own storage, zero maintenance planned); IUM's focus moves to the famous-platforms tranche: #63 rclone bridge → #59 native Google Drive → #64 TeraBox tranche → #60 YouTube → #61 MEGA → #62 socials spike.

Vault usage continues: `.secrets/feishu.py` (token/append/read, masked echo) + the vault doc — PAT stored there, handshake re-verified after every recycle (9 restores and counting)."""
})
print("comment posted on #56")

# Update the M10 milestone description with the new order
api("PATCH", "/repos/%s/milestones/9" % REPO, {
    "description": """M10 — The famous platforms via the rclone bridge (+ native depth). Order: #63 rclone bridge (74 backends day one, rehearses with NO credentials — wire facts W1-W9 pinned LIVE 2026-11-05) -> #59 native Google Drive (depth) -> #64 TeraBox tranche (fork lane + census) -> #60 YouTube -> #61 MEGA (bridge covers it functionally; native = depth) -> #62 socials spike. Feishu (owner clarification, chat 2026-11-05): VAULT-ONLY, adapter lane closed. Every lane obeys the loop: wire facts rehearsed LIVE, design note, RED, adapter over the same trait, journaled probe event, live drill byte-exact, full gates. M9 (#57, #58) runs first.""",
})
print("milestone M10 description updated")
