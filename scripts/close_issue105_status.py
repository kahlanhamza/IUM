#!/usr/bin/env python3
"""Insert the #105 closer row into STATUS.md's session log (newest-first)."""
P = "STATUS.md"

ROW = (
    "| 2026-11-11 (the pre-release accuracy sweep, Closes #105) "
    "| **The owner's release bar enforced — every claim wire-fact-checked, provider research from the providers' own official docs.** "
    "The audit (issue body = the receipts) found three drift classes and the lane fixed all three. "
    "(A) CORRECTNESS: the guided region slot never reached the SigV4 scope — `submit()` sent only `{path,size,adapter,endpoint}` while the job-time signer reads `env IUM_S3_REGION → vault region → us-east-1` (the draft is never consulted), so a guided Wasabi `eu-central-2` pick was signed `us-east-1` and refused (Wasabi validates scope-vs-host; their own docs say cross-region is rejected). "
    "Fix, app-layer per ADR-012: guided submit writes the region through to the vault (the documented partial merge — absent field = keep, keys untouched) + the #81 hot-apply rebuilds the lane BEFORE `createJob`; custom mode writes nothing (pinned); an explicit `IUM_S3_REGION` env still wins (operator intent). The write-through also fixes share-link presign (same env→vault chain) for the guided destination. "
    "RED comment 6797741553 (pin failed at HEAD `100dfbf`); GREEN on `d80655a`. "
    "(B) REGISTRY vs OFFICIAL DOCS (all fetched 2026-11-11): Filebase's general S3 endpoint is **s3.filebase.io** (their S3 API overview; `.com` is the IPFS-backed endpoint) — template+sample+pin moved; Wasabi 8→**16** regions (docs.wasabi.com service-URL table, updated 2026-03-06: +us-central-1, ca-central-1, eu-west-1/2/3, eu-south-1, ap-northeast-1/2); DO Spaces 6→**13** (official availability page: +sfo2, lon1, tor1, blr1, atl1, ric1, mkc1); B2 **+us-east-005** (the region in Backblaze's official S3 example) and **−us-west-003** (uncorroborated this pass — the recipe now says exactly which codes the picker carries and routes the rest through Custom URL); Scaleway/R2/MinIO verified, unchanged. The engine `parse_target` drift guard re-proved every new sample (7/7). "
    "(C) DOCS: the private USERGUIDE §1/§2 promised public `.attestation.1.jsonl` + SBOM and the README Security claimed a public SBOM — **false since #102** (wire-fact: the public v0.2.2 carries 15 assets, zero provenance); all three now say checksums + minisign signatures; badge 329+163 → **346+191** (both suites re-run this session); Highlights + transports table gained the TeraBox·Feishu·MEGA vault lanes (the README contradicted its own USERGUIDE §5.2); examples v0.2.0→v0.2.2; the STATUS header (stuck at M14) refreshed. "
    "GATE: full battery GREEN on the committed tree `e1a3650` (fmt + clippy -D warnings + core 346 incl. trybuild + drift guard + doc lint + tsc/vite/vitest **191/191** with a11y). "
    "HONESTY NOTES: (1) B2's CA East (Toronto, launched 2025-03-06 per Backblaze's blog) is NOT yet in the picker — the exact S3 code was not corroborated by an official page before the web budget ran out; recorded as a follow-up the moment it is confirmed. (2) The guided-region fix changes vault state on submit (documented in USERGUIDE §5.3). (3) RELEASE-LANE ITEMS recorded on the issue: all six product shots pre-date the provider picker (re-shoot at the cut, shot 02 driving the guided flow); the distro README/USERGUIDE get v0.2.3-true wording at the cut (#104 scope-honesty rule). "
    "| gate GREEN on `e1a3650`; vitest 191/191; core 346/346; drift guard 7/7; RED→GREEN pinned | d80655a + c31c4fe + e1a3650 |"
)

with open(P, encoding="utf-8") as f:
    content = f.read()

marker = "|---|---|---|---|\n"
idx = content.index(marker) + len(marker)
content = content[:idx] + ROW + "\n" + content[idx:]

with open(P, "w", encoding="utf-8") as f:
    f.write(content)
print("row inserted")
