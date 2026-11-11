#!/usr/bin/env python3
"""Post the #108 spike verdict comment + file the two GO follow-up lanes.

Verdict basis (official sources only, fetched 2026-11-12):
- Dropbox OAuth guide (developers.dropbox.com/oauth-guide): refresh tokens via
  token_access_type=offline; PKCE for desktop; token TTL returned by /oauth2/token.
- Dropbox files.stone (dropbox-api-spec, official): sessions valid 7 days;
  incorrect_offset -> UploadSessionOffsetError.correct_offset (realign pattern);
  payload <= 150 MiB; max file 2^41-2^22 B; concurrent mode chunks multiple of 4 MiB;
  finish -> FileMetadata.
- Dropbox content-hash reference (dropbox.com/developers/reference/content-hash):
  4 MiB blocks, SHA256(SHA256(block_i) concat) — server-side verify hook.
- Microsoft Graph createUploadSession (learn.microsoft.com): chunks MUST be multiples
  of 320 KiB, recommended 5-10 MiB; nextExpectedRanges resume; session expiry extends
  per fragment; final PUT -> 201/200 + driveItem.
- Microsoft Graph hashes resource: quickXorHash is the ONLY hash guaranteed for both
  OneDrive for work/school and home; sha256Hash explicitly not supported.
- Microsoft Entra refresh-tokens + access-tokens pages: access tokens 60-90 min random
  (75 avg); refresh 90-day sliding for public clients; rotation-on-use; revocation
  causes enumerated.
"""
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


VERDICT = """## The spike verdict — GO for both, sequenced: **Dropbox first, OneDrive second.**

Research source discipline honored: every row below comes from the providers' own
official documentation (Dropbox developers docs + the official `dropbox-api-spec` stone
files; Microsoft Learn Graph/Entra pages). Fetched and read 2026-11-12.

### Dropbox — **GO** (follow-up: #109)

| Row | Official fact | Source |
|---|---|---|
| App registration | Dropbox App Console; scoped apps declare every scope; scope minimization matters ("requesting more scope than required could impact your app review process") | OAuth guide |
| Token mechanics | Short-lived access tokens (exact TTL returned by `/oauth2/token`); refresh tokens granted with `token_access_type=offline`; PKCE (S256) required for desktop-class clients | OAuth guide |
| Upload protocol | `upload_session/start` → cursor → `append_v2` → `finish(CommitInfo)` → **FileMetadata**. Sessions **valid 7 days**. Per-request payload ≤ **150 MiB**. Max file = 2⁴¹−2²² B (~2 TB). Concurrent mode: offsets/chunks multiple of **4 MiB** | `files.stone` (official spec) |
| Resume semantics | `incorrect_offset` error carries **`correct_offset`** — the server hands back the authoritative offset; exactly our TUS 409-realign pattern (#10), so `reattach` + drifted-confirm absorb is a known shape | `files.stone` |
| Verify story | **`content_hash`**: SHA-256 of the concatenation of per-4 MiB-block SHA-256s; server returns it on FileMetadata and explicitly invites client-side comparison ("verify uploaded contents… without downloading") — the strongest verify-before-declare hook of any provider we ship | content-hash reference |
| Maintenance liability | API v2 stable; wire contract versioned openly in `dropbox-api-spec`; 7-day sessions; the liability is owning the app registration (scopes, review) — bounded | OAuth guide + spec |

### OneDrive (Microsoft Graph) — **GO, second** (follow-up: #110)

| Row | Official fact | Source |
|---|---|---|
| App registration | Entra ID app registration; public-client desktop flow; publisher verification applies for multi-tenant distribution; tenant Conditional Access policies can restrict third-party apps — a real-world surface the bridge does not surface | Entra docs |
| Token mechanics | Access tokens **60–90 min, random (75 min avg)** — never hardcode; refresh tokens **90-day sliding** for public clients, **rotate on every use** (must persist the fresh refresh token after each refresh — our #105 vault write-through pattern), revocation causes enumerated (password change, revoke, sign-out…) | Entra refresh/access token pages |
| Upload protocol | Create upload session → PUT fragments — **every chunk MUST be a multiple of 320 KiB** (recommended 5–10 MiB) → resume via GET `uploadUrl` `nextExpectedRanges`; session expiry **extends per fragment**; final PUT → 201/200 + driveItem | Graph createUploadSession |
| Resume semantics | `nextExpectedRanges` = authoritative missing-ranges read-back (the reattach probe); the 320 KiB uniformity maps onto our prescribed-geometry channel (`ProbeReport::uniform_block`, the Feishu lane's shape) | Graph createUploadSession |
| Verify story | **`quickXorHash` is the only hash guaranteed for both OneDrive for work/school and OneDrive for home**; `sha256Hash` explicitly "isn't supported. Don't use." → verify requires a quickXorHash implementation (algorithm published by Microsoft) | Graph hashes resource |
| Maintenance liability | Graph v1.0 versioned with a breaking-change policy; tenant-policy variance; the quickXorHash dependency; short access tokens mean mid-job refresh paths get exercised in normal operation — more states, more tests | Entra + Graph docs |

### Why GO at all (the honest delta over the bridge)

The bridge authorize family (#82) already signs users in for these providers — native
buys exactly four things the bridge structurally cannot: (1) **verify-before-declare on
the provider's own integrity hash** (content_hash / quickXorHash) instead of bridge-level
commit honesty; (2) **journal-native reattach** through our adapter contract
(probe/upload_chunk/finalize/verify/reattach, ADR-002) rather than a bridged process
boundary; (3) **our own OAuth app identity** (scope minimization, branding, rate-limit
identity not pooled with every other bridge user); (4) **prescribed-geometry planning**
(uniform blocks per the provider's alignment rules, the #56/#13 channels).

### Sequencing + blockers

- **#109 Dropbox first** — cleaner contract, strongest verify story, fewer tenant-policy
  variables.
- **#110 OneDrive second** — needs the quickXorHash implementation + multi-tenant
  verification + the mid-job token-refresh states.
- Both are **BLOCKED-ON owner app registrations** (the #59/#61 pattern): Dropbox app
  console (client id; PKCE-only desktop flow), Entra app registration (client id +
  tenant config). No code moves until those exist.
"""

DROPBOX_BODY = """## Problem / user story

The #108 spike verdict (official-docs-only research, 2026-11-12) is **GO** for a native
Dropbox connector. Today Dropbox ships through the bridge authorize family (#82); native
buys verify-before-declare on the provider's own `content_hash`, journal-native reattach
through our adapter contract, our own OAuth app identity (PKCE, scope minimization), and
planning freedom.

## The lane (design-note-first, credential-blocked)

1. **Design note on this issue BEFORE any code** — the adapter contract mapping:
   `probe` = `upload_session/start` (cursor journaled, ADR-002); `upload_chunk` =
   `append_v2` with the offset dance; `finalize` = `finish(CommitInfo)`; `verify` =
   server `content_hash` vs locally computed (4 MiB blocks, SHA-256 of concatenated
   block hashes — documented algorithm); `reattach` = lookup cursor → `incorrect_offset`
   carries `correct_offset` (the #10 409-realign pattern). Token path: OAuth code +
   PKCE (S256) in the user's browser, refresh token (`token_access_type=offline`) into
   the #81 SecretStore; access-token TTL read from the wire, never hardcoded.
2. **RED evidence at HEAD** before implementation (compile-fail or behavioral, the
   usual contract).
3. **Wire-facts rehearsal LIVE** against the real API before the adapter lands (the
   docs/05 "Real data only" rule) — BLOCKED-ON the owner creating the Dropbox app in
   the App Console (client id; PKCE-only desktop flow; scopes:
   `files.content.read`/`files.content.write` minimum).
4. Session honesty: sessions valid 7 days (official); per-request payload ≤ 150 MiB;
   `too_large` cap ~2 TB surfaced as an honest adapter limit.

## Acceptance criteria

- [ ] Design note posted (contract mapping + the refresh-token loss UX contract)
- [ ] RED at HEAD posted on this issue
- [ ] Live wire-facts rehearsal with the owner-supplied app registration (masked logs)
- [ ] `adapter_dropbox` lands behind the standard gates; trybuild contract extended if
      the trait surface moves (it should NOT — pure new adapter, ADR-013-style)
- [ ] Verify refuses to declare Done without the `content_hash` match (verify-before-
      declare, ADR-004)
- [ ] Full gates green + CI verdict chased to completion
- [ ] Shell exposure rides the existing connect surface (the #82 pattern; OAuth
      backend allowlist gains `dropbox`-native when the bridge path is replaced —
      migration behavior decided in the design note)

## Out of scope

- OneDrive (own lane #110, sequenced second)
- Sharing links / team scopes / paper API
- Any credential relay — keys stay local (#81 seam only)

## Boundary register (docs/08 §4)

Entities: adapter registry (new adapter), journal (probe event carries the Dropbox
cursor), SecretStore (refresh token). Edges at risk: adapter↔wire (new — wire-facts
ledger required), verify↔declare (content_hash gate), reattach↔journal (cursor replay).
Pinning tests: `tests/verify_is_required.rs` (trait), the S3 drill shape for resume
semantics adapted to `incorrect_offset`, new `tests/adapter_dropbox_*` for the
offset-realign + verify-refuses-to-lie paths.

## Refs

#108 (the spike, verdict + official sources); #82 connect surface; #81 SecretStore;
#59/#61 the credential-blocked precedent; docs/05 "Real data only"; ADR-002/004/010/011.
"""

ONEDRIVE_BODY = """## Problem / user story

The #108 spike verdict is **GO for OneDrive, sequenced SECOND** (after #109 Dropbox).
Native buys verify-before-declare on `quickXorHash` (the ONLY hash Microsoft guarantees
across OneDrive for work/school and home; `sha256Hash` explicitly unsupported),
journal-native reattach, our own Entra app identity, and 320 KiB-uniform planning.

## The lane (design-note-first, credential-blocked, strictly after #109)

1. **Design note on this issue BEFORE any code** — contract mapping: `probe` =
   create upload session (uploadUrl journaled, ADR-002); `upload_chunk` = PUT fragment
   (EVERY chunk a multiple of **320 KiB** — the prescribed-geometry channel,
   `ProbeReport::uniform_block`, the #56 shape; recommended 5–10 MiB); `finalize` =
   the final PUT's 201/200 + driveItem; `verify` = server `quickXorHash` vs locally
   computed (Microsoft-published algorithm; implementation + vectors as a pure module,
   the #6 sigv4 pattern — independent reference FIRST); `reattach` = GET uploadUrl →
   `nextExpectedRanges` read-back. Token path: Entra public-client OAuth + PKCE in the
   browser; **access tokens 60–90 min random** — the adapter refreshes mid-job and the
   rotated refresh token is persisted through the SecretStore write-through (#105
   pattern) on EVERY refresh; refresh token 90-day sliding, revocation causes
   enumerated → the token-loss UX contract named in the design note.
2. **RED evidence at HEAD** before implementation.
3. **Wire-facts rehearsal LIVE** against the real API — BLOCKED-ON the owner creating
   the Entra app registration (client id + tenant config; publisher verification for
   multi-tenant).
4. Tenant-policy honesty: Conditional Access may block third-party desktop apps on some
   tenants — the error surfaces speak the #70 copy table, raw preserved.

## Acceptance criteria

- [ ] Design note posted (contract mapping + token-refresh state machine + token-loss UX)
- [ ] quickXorHash pure module with independent-reference vectors pinned BEFORE the
      adapter (the #6 discipline)
- [ ] RED at HEAD posted on this issue
- [ ] Live wire-facts rehearsal with the owner-supplied Entra registration (masked logs)
- [ ] `adapter_onedrive` lands behind the standard gates; verify refuses Done without
      the quickXorHash match (ADR-004)
- [ ] Full gates green + CI verdict chased to completion
- [ ] Shell exposure rides the connect surface (#82 pattern), migration from the bridge
      path decided in the design note

## Out of scope

- Dropbox (#109, first)
- SharePoint site drives beyond the user's own OneDrive
- Any credential relay — keys stay local

## Boundary register (docs/08 §4)

Entities: adapter registry (new adapter), journal (probe event carries uploadUrl +
expiry), SecretStore (refresh token, rotation-on-use write-through), pure hash module.
Edges at risk: adapter↔wire (new — wire-facts ledger), verify↔declare (quickXorHash
gate), reattach↔journal (uploadUrl replay + nextExpectedRanges), chunk-plan↔adapter
(320 KiB uniformity via the geometry channel, ADR-011/ADR-012 discipline). Pinning
tests: `tests/verify_is_required.rs`, new `quickxor_vectors` contract tests, new
`tests/adapter_onedrive_*` for the fragment-alignment + refresh-mid-job + verify paths.

## Refs

#108 (the spike, verdict + official sources); #56 (uniform-block precedent); #105
(vault write-through precedent); #81 SecretStore; #6 (pure-module + independent-
reference discipline); docs/05 "Real data only"; ADR-002/004/010/011/012.
"""


def main() -> int:
    # 1) post the verdict comment on #108
    comment = api("POST", "issues/108/comments", {"body": VERDICT})
    print(f"verdict comment: id {comment['id']} on #108")

    # 2) file the two follow-up lanes on milestone M11 (number 10)
    ms = api("GET", "milestones?state=all&per_page=50")
    mid = next((m["number"] for m in ms if m["title"].startswith("M11")), None)
    for title, body in (
        ("M11#2 The Dropbox lane: native OAuth + upload_session adapter (GO per #108, "
         "credential-blocked on the owner's app registration)", DROPBOX_BODY),
        ("M11#3 The OneDrive lane: native Entra OAuth + Graph upload-session adapter "
         "(GO per #108, sequenced second, credential-blocked on the owner's Entra app)",
         ONEDRIVE_BODY),
    ):
        issue = api("POST", "issues", {
            "title": title, "body": body,
            "labels": ["enhancement", "core"],
            **({"milestone": mid} if mid else {}),
        })
        print(f"filed: #{issue['number']} {issue['title'][:72]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
