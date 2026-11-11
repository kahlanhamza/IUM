#!/usr/bin/env python3
"""File the pre-release accuracy sweep issue (owner directive 2026-11-11:
'every small detail 100% correct, research from reliable sources, both
READMEs updated before release'). Wire-facts evidence inline."""
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()

TITLE = ("Pre-release accuracy sweep: the guided region never reaches the signing "
         "scope; the provider registry is stale against the providers' own 2026 "
         "docs; the private docs still promise public SBOMs")

BODY = """## Why now (owner directive)

Before the v0.2.3 cut: "every small detail or implementation is 100% correct, the research is from reliable sources, and the READMEs of both IUM and IUM-dist are updated where needed." This lane is the audit that found real drift — filed with the receipts, fixed in this lane.

## Design note (the audit's method)

Every claim below was wire-fact-checked against (a) the actual code paths and (b) the **providers' own official documentation pages** (fetched this session, cited per line) — no search-snippet truth. The distro surface was verified separately (#104): its README/USERGUIDE are v0.2.2-true and unauth-clean. **This issue is about the private repo.**

## Findings

### A. Correctness bug — the guided region never reaches the SigV4 scope

`CreateJobForm.submit()` sends only `{source_path, source_size, adapter, endpoint}`. The picked region slot shapes the derived *host* but nothing else. The job-time signer region is built in `lib.rs` as `env IUM_S3_REGION → vault("region") → "us-east-1"` — the job draft is never consulted.

Consequence: a user guided through the Wasabi preset with region `eu-central-2` gets endpoint `https://s3.eu-central-2.wasabisys.com/bucket` signed with scope `us-east-1` (or whatever sits in the vault from an earlier Settings save). Wasabi validates scope-vs-host region (their docs: "pick the same region as your bucket — cross-region calls are refused", and the SigV4 scope must match); the upload is rejected with an error the user cannot decode.

**Fix (app-layer, ADR-012-clean — provider knowledge stays DATA in the app, the engine stays provider-agnostic):** on a guided submit with a region slot, `credentialsSet("s3_multipart", { region })` (the vault `set` is a documented partial merge — absent field = keep — so keys are never touched) then `applyCredentials("s3_multipart")` (the #81 hot-apply rebuilds the lane without restart), then `createJob`. Precedence stays honest: an explicitly-set `IUM_S3_REGION` env still wins in `from_env` (operator intent); the vault write-through also makes the share-link presign (`lib.rs` cfg builder, same env→vault chain) sign with the region matching the destination the user actually picked.

RED first: a vitest pin asserting a guided Wasabi `eu-central-2` submit calls `credentialsSet` with `{region: "eu-central-2"}` + `applyCredentials` **before** `createJob` — fails at HEAD.

### B. Provider registry stale against the providers' own docs (all fetched 2026-11-11)

| Preset | Registry says | Provider's official page says | Action |
|---|---|---|---|
| Filebase | endpoint `https://s3.filebase.com` | **`s3.filebase.io`** — "Point any AWS SDK or S3 client at s3.filebase.io"; `.com` is the IPFS-backed endpoint (filebase.com/docs/s3-api/overview) | **Fix template + sample_url + vitest pin** |
| Wasabi | 8 regions | **15 regions** (docs.wasabi.com "Service URLs for Wasabi's Storage Regions", updated 2026-03-06): add us-central-1, ca-central-1, eu-west-1, eu-west-2, eu-west-3, eu-south-1, ap-northeast-1, ap-northeast-2 | Complete the list |
| DO Spaces | 6 regions | **13 regions** (docs.digitalocean.com Spaces availability): add sfo2, lon1, tor1, blr1, atl1, ric1, mkc1 | Complete the list |
| Backblaze B2 | us-west-001..004, eu-central-001..003 | Official format `https://s3.<region>.backblazeb2.com` confirmed; official docs use `us-east-005` in examples; CA East (Toronto) region launched 2025-03-06 (Backblaze blog) | Add `us-east-005`; add the CA East code once its exact S3 code is confirmed (picker works today; key_steps already points at the bucket's own endpoint) |
| Scaleway | fr-par, nl-ams, pl-waw | Matches official docs (s3.{region}.scw.cloud) | No change |
| R2 / MinIO | account-id host / self-hosted base | Matches official docs | No change |

The drift guard (`ium-core/tests/providers_registry.rs`) re-derives every sample URL from the JSON through the REAL `parse_target`, so JSON edits self-prove; the vitest pins that hard-code Filebase's `.com` sample get updated with the fix. The engine's signing scope for Filebase (no region slot) defaults to `us-east-1` — exactly Filebase's documented fallback ("If your tool or SDK rejects auto, use us-east-1").

### C. The private docs still promise artifacts the public release does not ship

Wire-fact: the public v0.2.2 release carries **15 assets** — 7 installers/updater files + 7 signatures + `SHA256SUMS.txt` + `latest.json`. No attestation, no SBOM (the #102 provenance sandbox: provenance-shaped files never go public). But:

- `USERGUIDE.md` §1: "you will find … a signed attestation (`.attestation.1.jsonl`) and software bill of materials (SBOM) for supply-chain auditing" — **false since #102**.
- `USERGUIDE.md` §2: "Teams that need a supply-chain-grade audit can additionally verify each artifact's signed attestation sidecar against its SBOM." — **false since #102**.
- `README.md` §Security: "Every release ships checksums and a software bill of materials" — **half false** (checksums yes, public SBOM no).

Also stale: README badge "329 core + 163 UI" (counts move in this lane — badge set from the actual re-run); README Highlights/transports table missing the Terabox · Feishu · Mega vault lanes (the private README is *inconsistent with its own USERGUIDE §5.2* and the distro table); USERGUIDE examples still say v0.2.0 (current release v0.2.2); STATUS.md header "Last updated" stuck at M14.

### D. Release-lane items (NOT this lane — recorded so they are not lost)

- All six product screenshots were re-shot 2026-11-10 11:53 (v0.2.1); the provider picker landed `e26a7cb` 2026-11-09:28. Every shot pre-dates the headline UX of v0.2.3 → re-shoot through the real harness at the cut, with shot 02 driving the guided picker flow.
- The distro README/USERGUIDE get their v0.2.3-true wording (picker/share-link now downloadable) **at the cut**, per the #104 scope-honesty rule.
- v0.2.3 cut choreography (ADR-007: CI green before tag) is its own lane after this one.

## Acceptance criteria

- [ ] RED evidence posted: guided-submit vitest pin fails at HEAD (region never reaches the vault/apply)
- [ ] Guided submit: vault region write-through + hot-apply before createJob; custom mode sends nothing
- [ ] providers.json: Filebase `.io`; Wasabi 15; DO 13; B2 +us-east-005 — every sample URL re-proven through the engine's `parse_target` (drift guard green)
- [ ] Vitest pins updated with the data; full local gates green on the committed tree
- [ ] Private README + USERGUIDE: no public-SBOM/attestation claims; v0.2.2 examples; badge = actual counts; transports table lists the vault lanes
- [ ] STATUS header refreshed; session row appended; push; CI verdict chased to green; closure comment with receipts
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": TITLE, "body": BODY, "labels": ["bug", "docs"]}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"filed #{out['number']}: {out['html_url']}")
