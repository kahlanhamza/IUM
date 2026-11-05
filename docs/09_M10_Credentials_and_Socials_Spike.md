# 09 — M10 wire facts: the socials spike verdicts + the credentials playbook

**Date: 2026-11-06 · Deliverable of issue #62 (the socials spike — research only, NO adapter
code from this issue) + the credentials asks that unblock #59 / #60 / #61 / #64.**
Method: web wire-facts pass with every load-bearing fact cited; primary vendor docs pinned
where reachable, vendor-blog claims marked secondary. Live code rehearsal (the loop's
preferred wire-fact source) is deliberately deferred per lane: these gates are
*organizational* (app review, cookies, OAuth consent) — the rehearsal that matters is the
owner completing the credential steps below, then the lane's own loop begins with the live
rehearsal as usual. Nothing in this doc substitutes for that rehearsal.

---

## 1. The verdicts (spike deliverable)

| Platform | Lane | Verdict | Owner credential needed | Native IUM adapter? |
| --- | --- | --- | --- | --- |
| Google Drive | #59 | **GO — unblock with one OAuth client** | Google Cloud project + Desktop OAuth client JSON | Yes (resumable, 308 = `reattach`) |
| YouTube | #60 | **GO — same client + Data API enabled** | (same client) + YouTube Data API v3 enabled | Yes — **no rclone backend exists**; native is the only path |
| MEGA | #61 | **GO — day one through the #63 bridge**; native lane stays research-track | MEGA account email + password (+2FA if on) | Bridge first (`rclone serve s3` with a `mega:` remote); native crypto lane later |
| TeraBox | #64 | **GO — fork lane + cookies** | TeraBox account + `ndus` cookie (+ `jsToken`) | Fork `serve s3` over the existing bridge; cookies are the fragile part |
| TikTok | #62 | **GO, phased** — private-post drills now, client audit before public posts | TikTok developer app (client key/secret) + `video.publish` | Native only (no rclone backend) |
| Instagram | #62 | **GO — owner-account scope needs NO Meta App Review** | Meta app + Instagram **Professional** account (linked Page) | Native only |
| Facebook | #62 | **GO — owner-Page scope needs NO App Review** | Same Meta app + a Page where the owner is admin | Native only |

The #63 rclone bridge (74 backends, closed 2026-11-05) already carries the storage
platforms among "the famous ones": Drive, OneDrive, Dropbox, Box, pCloud, Koofr, iCloud
Drive, Mega, S3 family — see the current backend list at <https://rclone.org/docs/#backend-list>.
**No YouTube, TikTok, Instagram or Facebook backend exists in rclone** — those lanes are
native-only by construction. **No TeraBox backend in upstream rclone** — the fork lane (#64).

---

## 2. TikTok (issue #62) — Content Posting API

Primary source: <https://developers.tiktok.com/doc/content-posting-api-get-started> (fetched 2026-11-06).

Wire facts pinned from the official page:

- **The wire**: `POST https://open.tiktokapis.com/v2/post/publish/video/init/` with
  `source_info.source = FILE_UPLOAD` (local file, chunked upload — `video_size` + `chunk_size`
  declared in the init) or `PULL_FROM_URL` (from a **verified domain/URL prefix** only).
  `GET /v2/post/publish/creator_info/query/` MUST be called first (the UX contract — the
  response's `privacy_level_options` governs which `privacy_level` the post may use; posting
  a level outside the creator's options fails).
- **The decisive gate**: *"All content posted by unaudited clients will be restricted to
  private viewing mode. Once you have successfully tested your integration, to lift the
  restrictions on content visibility, your API client must undergo an audit."* (verbatim,
  official page).
- Requirements stack to direct-post: registered app at developers.tiktok.com → add the
  **Content Posting API** product → enable the **Direct Post** configuration → get approval
  for the **`video.publish` scope** → the target user authorizes the scope.
- Token reality (secondary, <https://postproxy.dev/blog/how-to-post-to-tiktok-via-api>,
  consistent with the official docs' token pages): access token ~24 h, refresh token ~365 d;
  ~15 posts/day/creator; ≤ 4 GB video; caption ≤ 2,200 chars.
- **Go/no-go answers**:
  1. *Can the owner get approved?* The scope grant + Direct Post config are obtainable by
     any developer account; the AUDIT (to make posts public) is the real gate. Secondary
     source (Blotato, <https://www.blotato.com/blog/tiktok-api-pricing>) claims TikTok will
     not audit personal-use apps — **unverified, treat as a risk, not a fact**. Phase the
     lane so this never blocks the engineering: unaudited clients can still post (PRIVATE) —
     which is enough for the full upload pipeline, status polling and drill work.
  2. *Verify semantics vs invariant #1*: `PUBLISH_COMPLETE` ≠ a visible post (TikTok
     moderates after publish). Verify = the status-poll honest variant — the same shape
     already planned for #60; journaled, never byte-claimed. Acceptable.
  3. *Content policy*: no watermarks/promo overlays; branded-content / AI-generated
     disclosure booleans ride `post_info`.

**Owner ask (Phase 1, ~15 minutes)**: create an app at <https://developers.tiktok.com/apps>,
add the Content Posting API product, enable Direct Post, note the client key + secret into
`.secrets/tiktok_app.json` (see §8). Phase 2 (public posts) = request `video.publish`
approval and the client audit when the pipeline is proven.

---

## 3. Instagram (issue #62) — Graph API Content Publishing

Primary source: <https://developers.facebook.com/documentation/instagram-platform/content-publishing>;
access realities: <https://www.blotato.com/blog/instagram-api-pricing> (secondary, vendor).

Wire facts pinned:

- **Instagram Basic Display API is DEAD** (shut down 2024-12-04). The only official path is
  the **Instagram Graph API with Instagram Login**, which requires an **Instagram
  Professional (Business or Creator) account**. Personal accounts have zero API access.
- **The wire**: container-then-publish two-step — `POST /{ig-user-id}/media` (creates a
  media container from a publicly-reachable media URL; Reels ride the same container flow)
  → poll the container until `status_code = FINISHED` → `POST /{ig-user-id}/media_publish`.
  Server-side processing between container and publish means **no byte-level verify** —
  the destination is a post/reel, verify = container/publish status poll (the honest
  variant, journaled).
- **Rate**: 100 API-published posts per 24 h moving window (official page); Meta's Business
  Use Case rate limit otherwise scales with the account's impressions.
- **The review reality (the spike's decisive finding)**: with the app in **Development
  mode**, any user **with a role on the app** (developer/tester/admin) can grant any
  permission — no App Review needed. Meta App Review + Business Verification are required
  only for **Advanced Access** (users with no role on the app). For IUM's owner-account
  publishing: **zero review wall**. Public distribution later = the review + verification
  path.
- A Page with pending **Page Publishing Authorization (PPA)** blocks publishing until done
  (official note) — keep the linked Page clean.

**Owner ask (~20 minutes)**: convert the Instagram account to Professional (Business or
Creator) → link/ensure a Facebook Page → create a Meta app at
<https://developers.facebook.com> → add the Instagram Graph API product → add the owner's
accounts as testers (roles) → `.secrets/meta_app.json` (see §8). One Meta app covers
Instagram **and** Facebook (§4).

---

## 4. Facebook (issue #62) — Pages API (video/reels)

Primary source: <https://www.breakreach.com/developers/facebook-api> (secondary vendor, but
every fact cross-checked against Meta's own Pages API docs; Graph API v26.0 since
2026-07-29).

Wire facts pinned:

- **Pages only** — personal profiles (since 2018) and Groups (since 2024-04) are closed to
  apps. IUM's Facebook lane = publish video/reels to a **Page**.
- **Permissions**: `pages_manage_posts` (+ `pages_read_engagement`, `pages_show_list`) on a
  **Page access token**. Exchange the user token for a long-lived one (~60 d), then
  `GET /{user-id}/accounts` — the Page tokens it returns **do not expire**.
- **Video upload**: resumable video upload sessions exist on `/{page-id}/videos` (start →
  transfer → finish, session-shape like Drive's 308 pattern — the lane's live rehearsal will
  pin the exact three-phase wire). Reels have a separate flow with ~30 posts/day.
- **Rate limit**: 4,800 calls × the Page's engaged users per 24 h.
- **Review reality**: identical to Instagram — Development-mode + a role on the app =
  any permission grantable, no review. Public distribution = App Review + Business
  Verification.

**Owner ask**: same Meta app as Instagram (one app, both lanes) + the owner's Page ID into
`.secrets/meta_app.json`.

---

## 5. The Google family — ONE credential unblocks #59 AND #60

Primary sources: <https://developers.google.com/workspace/drive/api/guides/api-specific-auth>
(scope categories), <https://developers.google.com/youtube/data/guides/quota_and_compliance_audits>
(quota structure), <https://support.google.com/cloud/answer/13463073> (verification rules),
<https://stackoverflow.com/questions/69541148/googledrive-7-days-refresh-token> (testing-mode
token expiry).

### 5.1 The steps (owner, ~20 minutes, one project covers both lanes)

1. <https://console.cloud.google.com> → create a project (e.g. `ium`).
2. **APIs & Services → Library**: enable **Google Drive API** (for #59) and **YouTube Data
   API v3** (for #60) on that project.
3. **APIs & Services → OAuth consent screen**: User type **External**; app name IUM; add
   the owner's Gmail as a **test user**.
4. **Credentials → Create credentials → OAuth client ID → Application type: Desktop app** →
   download the JSON → save as `.secrets/gdrive_oauth_client.json` (see §8). This ONE
   client serves both lanes; #60 only additionally needs the `youtube.upload` scope
   requested at consent time.

### 5.2 The facts that shape the design

- **`drive.file` is a NON-sensitive scope** (official Drive scopes table — the
  non-sensitive list is `drive.appdata`, `drive.install`, `drive.file`; `drive` full is
  restricted). Google's rule: *"If your app utilizes only non-sensitive scopes, it is not
  mandatory for your app to complete the app verification process"* (OAuth App Verification
  Help Center). IUM requests only `drive.file` for Drive → **no verification wall for the
  Drive lane**, even in production.
- **`youtube.upload` rides the same consent**; the YouTube Data API has its own quota
  structure: **default = 10,000 units/day combined, PLUS separate count-based allotments —
  100 `search.list` calls and 100 `videos.insert` uploads per day** (official quota &
  compliance audits page, fetched 2026-11-06). This is the 2026 restructure: `videos.insert`
  cost 1,600 units before June 2026 (secondary:
  <https://www.activepieces.com/blog/youtube-api-quota-exceeded-how-to-fix-and-avoid-it>).
  100 uploads/day default is comfortable for owner use; the audit & quota-extension form
  (<https://support.google.com/youtube/contact/yt_api_form>) exists if the lane ever needs
  more.
- **The 7-day trap**: an OAuth app in **Testing** status gets its refresh tokens expired
  every **7 days** (rclone users live this: forum.rclone.org/t/7-day-reconnect-of-google-drive-oauth/50148).
  Escape: **publish the consent screen to Production**. With only non-sensitive scopes
  (`drive.file`) that requires NO verification — the app just shows the standard consent
  dialog. Do this when the lane goes real; Testing mode is fine for the first rehearsal.
- **The old private-lock**: videos uploaded by unverified API projects were historically
  forced private; the current API revision history states they are **no longer restricted to
  private viewing mode** (documented via <https://stackoverflow.com/questions/80009624/>;
  the historical rule remains in some older guides). The lane's live rehearsal will pin the
  actual behavior on the owner's channel.

---

## 6. MEGA (issue #61) — the bridge already carries it

Primary source: <https://rclone.org/mega> (fetched 2026-11-06).

- The **rclone `mega` backend** exists (inside #63's 74-backend bridge): config keys
  `user`, `pass` (rclone-obscured), optional `2fa`, plus cached `session_id` / `master_key`.
  It rides the `go-mega` library; rclone's own page notes *"There doesn't appear to be any
  documentation for the mega protocol beyond the mega C++ SDK source code"* — which
  **confirms the #61 issue's honesty**: the native MEGA lane is the crypto-heavy one
  (client-side AES + ECDH, no official third-party protocol docs), best left research-track.
- **Immediate path**: owner gives a MEGA account; the #63 bridge serves it over S3 with
  zero IUM changes (`RCLONE_CONFIG_*` env injection, already implemented). The lane's
  multipart/verify semantics were rehearsed in W4–W7 of the bridge wire facts.
- **Owner ask (~5 minutes)**: a MEGA account (email + password; if 2FA is enabled, the
  rotating code is needed at first login) → `.secrets/mega.json` (see §8).

### 6.1 LIVE REHEARSAL (2026-11-06, real credentials, `scripts/wirefacts61/probe_mega_bridge.sh`)

The owner's account works; the bridge carries it end-to-end **today**:

- **W1 login OK** (env-injected `RCLONE_CONFIG_MEGA_TYPE/USER/PASS`, pass via `rclone obscure`
  at runtime); root listing fine.
- **W2 rclone-wire roundtrip**: 1 MiB `copyto` → `cat` **byte-exact**.
- **W3 topology wire fact (the rehearsal caught it)**: `serve s3` parses the FIRST URL path
  segment as the bucket — serving `mega:ium-drill` directly mis-parses (`/mp.bin` → bucket
  `mp.bin` → `NoSuchBucket`, `Resource: mp.bin`); a bucket-less backend must be served from
  its **ROOT** with a top-level folder as the bucket. Corrected topology: `serve s3 mega:`,
  bucket = `ium-drill`.
- **W4 single PUT 200 + GET-back byte-exact** over the sigv4 wire on the corrected topology.
- **W5–W7 multipart FULLY GREEN on the LIVE backend**: `POST ?uploads` → real UploadId;
  2 × 5/4 MiB part PUTs → real per-part ETags; complete → `CompleteMultipartUploadResult`
  (ETag `-2` suffix); **9 MiB reconstructed byte-exact from MEGA**.
- Cleanup: `rclone purge` removed the drill dir — account left clean.

**Verdict: MEGA is a working IUM destination as of today** (bridge carrier). The native
crypto lane stays research-track exactly as the issue anticipated.

---

## 7. TeraBox (issue #64) — cookies, not an API

Primary sources: <https://github.com/Pahadi10/terabox-upload-tool> (the community upload
protocol), <https://github.com/saahiyo/terabox-gateway> (cookie recipe), both fetched
2026-11-06.

- **No official public API.** The community protocol authenticates with **browser cookies**:
  `ndus` (the session cookie — the one load-bearing value), plus `jsToken` (read from any
  `api/*` request in the browser's Network tab) and `appId` (community default `250528`).
  Some flows also read `csrfToken` / `browserid` (hass-terabox).
- **The upload wire** (community-pinned, to be rehearsed live on the fork lane):
  `precreate` (path + size + MD5) → chunked `superfile` PUTs → `create`/finalize. MD5 of the
  source is part of the protocol — IUM's journal already computes hashes for verify, so the
  wire maps cleanly onto the adapter contract.
- **Fragility (honest)**: cookies **expire periodically**; when auth errors start, the
  owner re-extracts a fresh `ndus`. The fork lane (bclone / rclone-extra, pinned on #64)
  carries this same cookie auth inside rclone-fork remotes — IUM unchanged either way.
- **Owner ask (~10 minutes)**: register/log in at <https://www.terabox.com> → F12 →
  Application → Cookies → `https://www.terabox.com` → copy the `ndus` value (+ `jsToken`
  from a Network-tab request) → `.secrets/terabox.json` (see §8).

### 7.1 LIVE REHEARSAL (2026-11-06, real cookies, `scripts/wirefacts64/` — the FULL protocol is now pinned)

The owner's cookies authenticate, and the complete write+read flow was driven LIVE
(2 MiB probe file, roundtrip **BYTE-EXACT**, account left clean). Nine wire facts, each
load-bearing:

- **W1 the API surface is `dm.terabox.com`, NOT `www`**: identical cookie jar —
  `www.terabox.com/api/*` answers `errno -6 "user not login"`; `dm.terabox.com/api/*`
  answers `errno 0`. Every community tool that targets `www`/`1024terabox` surfaces a
  different regional deployment (probed: `c-jp`/`data`/`up` hosts answer
  `31045 "user not exists"` — wrong deployment for this account).
- **W2 account facts**: quota `errno 0` — 30 GB permanent + 994 GB temporary (promo to
  2026-12), ~1 TB total, ~5 MB used.
- **W3 `jsToken` is SELF-SERVE**: it is embedded in the logged-in main page HTML
  (`fn("<128-hex>")` URL-encoded) — extractable with the SAME ndus cookie, no owner
  Network-tab step needed. Without it, all WRITE endpoints answer `errno 4000023
  "need verify"` (risk control); with it, they pass.
- **W4 `precreate` needs `autoinit=1`** to return the `uploadid` (63-char base64, embedding
  the client IP — session-bound). Without it: `return_type 1` and NO uploadid (the dead end
  the first probe hit).
- **W5 `locateupload` assigns the per-session upload host**:
  `GET dm.terabox.com/rest/2.0/pcs/file?method=locateupload` →
  `{server: ["dm1-cdata.terabox.com","dm2-cdata.terabox.com","kul-cdata.terabox.com"],
  client_ip, expire: 600}`. The fixed community host (`c-jp...`) is deployment-wrong
  (W1); the assigned host MUST be used.
- **W6 `superfile2` is a MULTIPART-FORM upload** (`file` field, `uploadsign=0`,
  `partseq=0`) on the assigned host → **HTTP 200 with the part's MD5 echoed back —
  matched our local MD5 exactly** (a built-in integrity signal for the adapter's verify).
- **W7 `create` (with `rtype=1`) finalizes**: `errno 0`, `fs_id` returned; `list` shows the
  file (name + size). `filemanager?opera=delete` removes it (`errno 0`) — cleanup works.
- **W8 download = PCS classic**: `GET dm.terabox.com/rest/2.0/pcs/file?method=download&path=…`
  → **302 to a signed CDN URL** (`kul-ddata.terabox.com/file/…`, `sign`/`pkey`/`expires`)
  → bytes. The `/api/download` and `filemetas` shapes returned `errno 2` / empty `info` —
  dead ends recorded for the adapter author.
- **W9 THE ROUNDTRIP**: 2 MiB uploaded (P1→P7), downloaded back through the signed URL,
  **byte-exact (cmp clean)**, then deleted — the account ends clean.

**Verdict: the community web protocol is directly adapter-implementable** — the wire maps
cleanly onto the IUM contract (precreate=init/session, superfile2=chunks with per-part MD5
echo, create=finalize, method=download=verify stream, filemanager=cleanup). **No fork
build is required for a native TeraBox adapter**; the fork lane (bclone/rclone-extra)
drops to optional/backup. The cookie-refresh fragility (W-known: `ndus` expires) remains
the one operational dependency — the adapter surfaces it as `AuthRequired` (the honest
case: reattach cannot fix dead credentials; the owner re-extracts the cookie).

### 7.2 IMPLEMENTED (2026-11-06, same day as the rehearsal — `adapter_terabox`, Closes #64)

The design note (D1–D6, issue comment 6668170486) + RED (compile-fail contract, comment
6668184038) → the native lane. Wire facts added by the multipart/verify probes
(`probe_terabox_multipart.sh`, `probe_terabox_verify.sh`):

- **W10 multi-slice PROVEN**: 3-entry `block_list` (4+4+1 MiB) → errno 0; 9 MiB roundtrip
  byte-exact → the lane PRESCRIBES uniform 4 MiB + exact tail (the #56 machinery).
- **W11 out-of-order partseq ACCEPTED** (partseq 1 before 0 → 200 + MD5 echo match).
- **W12 re-PUT of a done partseq is IDEMPOTENT** (200, same MD5) — crash-window re-sends safe.
- **W13 the verify route is PCS `method=download` ONLY**: `filemetas` dlink is UNRELIABLE
  on multi-slice files (no dlink field), `/api/download` answers errno 2 — dead ends recorded.

Deltas pinned by the implementation: errno -6 / 4000023 map to **AuthRequired** (not
ExpiredSession — reattach cannot fix dead credentials); the upload host (W5, expire 600 s)
is NEVER journal truth — reattach leaves it empty and the first part re-derives via
locateupload (a READ); per-slice MD5s are recomputed per call over the journal-immutable
source instead of bloating the token. LIVE drill green 8.79 s: 9 MiB / 3 slices,
out-of-order parts + a W12 re-send, fresh-adapter reattach, create → fs_id (twice —
finalize-once idempotence), download+sha256 byte-exact, cleanup delete. Suite 268.

---

## 8. The `.secrets/` contract (how every credential lands)

Same discipline as the existing `.secrets/gh_token`: gitignored + `.git/info/exclude`
belt-and-braces, `chmod 600`, never committed, never printed. The engine reads these files
at runtime (or the equivalent env names); the repo reads only their SCHEMAS:

| File (owner creates) | JSON shape | Serves |
| --- | --- | --- |
| `.secrets/gdrive_oauth_client.json` | Google's downloaded `client_secret.json` verbatim | #59 + #60 |
| `.secrets/mega.json` | `{"user": "...", "pass": "...", "2fa": "..."}` (2fa only if enabled) | #61 via bridge |
| `.secrets/terabox.json` | `{"ndus": "...", "jsToken": "...", "appId": "250528"}` | #64 |
| `.secrets/tiktok_app.json` | `{"client_key": "...", "client_secret": "..."}` | #62 Phase 1 |
| `.secrets/meta_app.json` | `{"app_id": "...", "app_secret": "...", "ig_username": "...", "page_id": "..."}` | #62 IG + FB |

The vault (Feishu document, per the owner's VAULT-ONLY directive) gets the research notes
of each lane; the credentials themselves stay in `.secrets/` on the machine.

---

## 9. What this changes on the board

**Owner directive (2026-11-06, chat): Google Drive, YouTube and the social-media lanes are
DEFERRED to later updates.** The M10 order is therefore re-sequenced: **#61 MEGA and #64
TeraBox first** (both credentials now in hand and both wires PROVEN live the same day —
§6.1 and §7.1), native TeraBox adapter as the next implementation rung; #59/#60/socials
wait for the owner's go.

1. **#62's spike deliverable is this document** — per-platform wire facts + the three
   go/no-go criteria answered. The issue closes with it; the ADAPTER work for TikTok/IG/FB
   becomes its own filed lanes once the owner's Meta + TikTok apps exist (Phase 1 = private
   / owner-scope publishing; review walls only gate public distribution later).
2. **#59 + #60 unblock with ONE owner action** (§5.1) — DEFERRED by the owner; the playbook
   stays on the shelf for the day they say go.
3. **#61: bridge path PROVEN LIVE** (§6.1) with the owner's credentials — MEGA works
   through IUM today; the native crypto lane stays research-track.
4. **#64: the full community protocol PROVEN LIVE** (§7.1) — a native TeraBox adapter is
   implementable straight from the nine wire facts; the fork probe drops to backup.
5. **Priority (re-sequenced by the owner)**: #64 native adapter (wire facts complete) →
   #61 bridge integration wiring (credentials → `RCLONE_CONFIG_*` seam) → then the
   deferred lanes when the owner says go.

## 10. Source-quality honesty

Primary vendor pages pinned: TikTok get-started, Meta Content Publishing docs, Google Drive
scopes, Google quota & compliance audits, OAuth verification Help Center, rclone /mega,
rclone backend list. Secondary (vendor-blog / community) claims, used only where no primary
page states the figure: TikTok token lifetimes + ~15 posts/day (postproxy), the
personal-use-audit risk (blotato), the videos.insert 1,600→quota restructure date
(activepieces), the TeraBox protocol shape (Pahadi10 / terabox-gateway / hass-terabox —
community libraries, no official spec exists to pin). Every secondary claim is re-provable
at the lane's own live rehearsal, which remains the loop's actual gate.
