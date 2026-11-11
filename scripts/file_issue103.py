#!/usr/bin/env python3
"""File the regular-user destinations lane (owner-ratified in chat, 2026-11-11).

The owner's words: regular users go to "a website X with an upload button" and
cannot obtain the direct destination URLs (endpoint/bucket/keys) the engine
speaks — they don't know how to leverage IUM or whether it would help. The
survey (this session) found: a raw per-job URL field, a one-line keys note,
NO S3 test arm in probe_credentials, NO provider presets, NO post-upload link.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from file_issue import api  # the shared token-reading helper (never prints the PAT)

TITLE = (
    "Regular-user destinations: provider presets, guided key setup, "
    "an S3 Test button, and the shareable link"
)

BODY = """## The problem (owner, 2026-11-11)

> our engine supports S3 compatible urls destinations, and other destinations,
> but the issue is regular users cannot get those direct destination urls,
> they usually only go to a website X and they have like upload button and
> stuff, and they don't know how to leverage our software, or even if our
> software would help.

Three walls, measured on the current tree:

1. **Vocabulary** — the create form asks for a raw `endpoint URL`
   (`CreateJobForm.tsx`, id `upload-endpoint`). Regular users think in
   websites ("Backblaze", "Cloudflare R2"), not path-style object URLs.
2. **Bootstrap** — the S3 lane's only help is one schema note
   (`vault.rs`: "Any S3-compatible endpoint (AWS R2 MinIO SeaweedFS…)"). Every
   provider hides its key console somewhere different; nothing tells the user
   where to click.
3. **Outcome** — a completed upload produces no link. `probe_credentials`
   honest-catchall (`lib.rs:847-854`) even says the S3 lane "verifies itself
   on the first upload" — the user discovers a wrong URL only after
   configuring a job. And there is no presign anywhere (`sigv4.rs` signs
   upload-path requests only), so a user who wanted "a link to send someone"
   gets nothing.

## The lane (owner-ratified, tiered)

**Tier 1 — provider presets + guided keys (app-layer, zero engine changes):**
- A single JSON registry (`ium-app/src/data/providers.json`), one source of
  truth: id, label, adapter, endpoint template (`{region}`/`{account_id}`
  slots), region lists, key-console deep link, per-provider key steps, and a
  `sample_url` per preset.
- The create form gains a provider picker (`upload-provider`): pick a
  provider, fill bucket (+ region/account slot), the endpoint derives live.
  "Custom URL" stays the default — power users keep today's raw field, the
  e2e seams (`upload-*` ids) stay byte-compatible.
- Each preset renders "Get your keys ↗" + a 3-click key recipe.
- Launch presets: Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean
  Spaces, Filebase, and Nextcloud (WebDAV). Honest notes for the rest.

**Tier 1.5 — the S3 Test button:**
- `S3MultipartAdapter::check_credentials(endpoint)`: zero-byte
  `CreateMultipartUpload` + best-effort `AbortMultipartUpload` on a probe key
  (`ium-probe/<ts>.bin`) — never touches a user key, never completes anything.
- `probe_credentials` grows an `s3_multipart` arm (drafts in memory, no
  session orphaned — same contract as #81); the S3 lane schema gains an
  optional non-secret `endpoint` field the Test drives against.

**Tier 2 — the shareable link:**
- `sigv4::presign` — query-string SigV4 (the presigned GET shape:
  `X-Amz-Algorithm/Credential/Date/Expires/SignedHeaders/Signature`),
  expires clamped to the S3 spec (1 s..604800 s).
- `share_link(job_id, ttl_hours)` shell command: engine-owned
  `parse_target` (no TS reimplementation), vault credentials, presigned GET —
  works for private AND public buckets; nothing is ever auto-published.
- UI: "Copy share link" on finished S3 jobs (Inspector), TTL fixed 24 h v1.

## Doctrine guards

- ADR-012 (new): provider knowledge is DATA in the app; the engine stays
  provider-agnostic (adapters declare, planner obeys — ADR-011 untouched).
- No relay service ever holds user credentials (the zero-telemetry posture).
- The raw URL field never disappears (power users are today's users).

## The test bar (owner: "must really cover all tests… shouldn't have any issues")

1. **Registry × engine**: a Rust integration test feeds EVERY preset's
   `sample_url` through the REAL `parse_target` — the registry cannot drift
   into engine-invalid URLs (plus TS schema tests: unique ids, registered
   adapter names, https key pages, template/region coherence).
2. **Presign vectors**: SigV4 presign pinned against an independently
   computed reference (same triple-source discipline as #6), plus expiry
   bounds, determinism, and credential-scope encoding cases.
3. **Probe wire**: a local mock S3 (the `common/tus_mock.rs` precedent)
   pins Create→Abort happy path, 403 reject, 404 bucket, transport refusal.
4. **Shell arm**: missing-field and bad-endpoint drafts answer honest
   `probe_missing` sentences; the arm is thin over the tested adapter.
5. **UI**: provider picker component tests (derive, restore-custom, keys
   link, hint copy), share-link gate test (`canShare`), api contract type.
6. **The full local battery** (CI is billing-blocked — local evidence carries
   this lane): fmt, clippy -D warnings, core full tests, doc, tsc strict,
   vitest, the vault pure-std lane. e2e contract intact by construction
   (defaults identical); the WebDriver lane rides CI.

## Acceptance

- [ ] providers.json + loader + schema tests (RED first)
- [ ] create-form picker + keys guidance, custom mode byte-identical
- [ ] S3 `check_credentials` + mock-S3 wire tests + settings `endpoint` field
- [ ] `probe_credentials` s3 arm
- [ ] `sigv4::presign` + vectors
- [ ] `share_link` command + Inspector copy-link moment
- [ ] USERGUIDE §5 rewrite, README providers list, ADR-012, CHANGELOG
- [ ] full battery GREEN locally; CI chase recorded (billing block standing)
"""

existing = api("GET", "issues?state=all&per_page=100")
if existing[0] == 200:
    for i in existing[1]:
        if i["title"] == TITLE:
            print(f"issue: EXISTS #{i['number']}")
            sys.exit(0)

status, created = api("POST", "issues", {"title": TITLE, "body": BODY, "labels": ["product", "ux"]})
print(f"issue: http-{status} #{created.get('number')}") if status == 201 else print(
    f"issue: FAILED http-{status}: {created}"
)
sys.exit(0 if status == 201 else 1)
