#!/usr/bin/env python3
"""File milestone M3 + the spike-derived issues (#6-#9) for IUM.

Reads the PAT from /home/z/my-project/.secrets/gh_token (never printed).
Idempotent: existing open issues are matched by exact title and skipped;
the milestone is matched by title.

Usage: python3 scripts/file_m3.py
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN_PATH = Path("/home/z/my-project/.secrets/gh_token")

MILESTONE = {
    "title": "M3: Real transports",
    "description": "docs/04_Roadmap.md M3 — S3 multipart vs MinIO + TUS 1.0.0 + kill-9 "
    "resume drill. DoD: end-to-end uploads of >100 MiB with forced mid-body kills; "
    "zero corruption on resume; honest Done only after adapter-verified completion.",
}

ISSUES = [
    {
        "title": "sigv4: AWS Signature V4 signer (pure, offline)",
        "labels": ["core"],
        "body": (
            "## Problem\n"
            "The S3 multipart adapter (#7) is blocked on AWS Signature Version 4 "
            "signing. Pulling `aws-sdk-s3` for ~6 operations would drag in the whole "
            "AWS common runtime; hand-rolling keeps the dep surface at `hmac` + `sha2` "
            "+ `hex` and is fully provable offline (docs/07 §5).\n\n"
            "## Acceptance criteria\n"
            "- `ium-core/src/sigv4.rs`: canonical request → string-to-sign → derived "
            "signing key → `Authorization` header (path-style, region-scoped)\n"
            "- modes: `UNSIGNED-PAYLOAD` for streaming bodies; real SHA-256 for small "
            "bodies (Create/Complete XML)\n"
            "- official AWS SigV4 test-suite vectors ported as unit tests (offline, "
            "deterministic)\n"
            "- deps limited to `hmac`, `sha2`, `hex` (+ existing); no network code in "
            "this module\n"
            "- fmt + clippy `-D warnings` clean; gates green\n\n"
            "## Refs\n"
            "docs/07_M3_S3_Multipart_Spike.md §5; blocks #7"
        ),
    },
    {
        "title": "adapter_s3: multipart adapter core (probe → UploadPart → finalize+verify)",
        "labels": ["core"],
        "body": (
            "## Problem\n"
            "First real transport: S3 multipart implementing `TransferAdapter` against "
            "MinIO (docs/07 §1–§4). The engine contracts are already S3-shaped "
            "(CHUNK_MIN = 5 MiB = S3 min part size; `adapter_token` doc names part "
            "numbers) — this issue stress-tests them for real.\n\n"
            "## Acceptance criteria\n"
            "- `ium-core/src/adapter_s3.rs`, registry name `s3_multipart`, "
            "capabilities `RESUMABLE | DYNAMIC_CHUNKS`\n"
            "- probe → `CreateMultipartUpload` (session_token = UploadId, "
            "max_chunk_bytes = Some(5 GiB))\n"
            "- upload_chunk → `UploadPart` with `partNumber = idx+1`, "
            "UNSIGNED-PAYLOAD, ETag → `adapter_token = \"N:etag\"`; bytes_confirmed "
            "= full span or Err (parts are atomic)\n"
            "- verify = finalize-if-needed + verify (**ADR-010 proposal**): ListParts "
            "reconcile → CompleteMultipartUpload → HEAD CL == source_size; "
            "NoSuchUpload-on-complete = already completed\n"
            "- 0-byte source → PutObject path (issue #5's `[(0,0)]` lease must still "
            "be one honest round-trip)\n"
            "- error mapping: NoSuchUpload → ExpiredSession; 5xx → Transient; 503 + "
            "Retry-After → Throttled\n"
            "- ListParts pagination loop (`part-number-marker`) implemented even "
            "though CHUNK_MAX × 1000 makes it unreachable in practice\n"
            "- env-gated tests (`IUM_S3_ENDPOINT` unset → skipped), green without "
            "MinIO locally\n\n"
            "Blocked by #6. Refs docs/07_M3_S3_Multipart_Spike.md §1–§5; ADR-010 "
            "proposal."
        ),
    },
    {
        "title": "planner/adapter contract: consume ProbeReport.max_chunk_bytes",
        "labels": ["core"],
        "body": (
            "## Problem\n"
            "The M3 spike found `ProbeReport.max_chunk_bytes` is discovered at probe "
            "but **never consumed** — no code path feeds it to the planner "
            "(docs/07 §2). The honest-contract story (\"adapter declares, planner "
            "obeys\") is currently discover-only.\n\n"
            "## Acceptance criteria\n"
            "- manager/planner: when `max_chunk_bytes = Some(m)`, planned chunk "
            "length never exceeds m — n raised to at least ceil(total/m) before "
            "planning (clamped by the existing chunk_min logic)\n"
            "- unit tests: cap binding (e.g. 10 MiB file, max 4 MiB → ≥3 chunks), "
            "cap not binding (max > CHUNK_MAX → unchanged), interplay with chunk_min\n"
            "- ADR-011: adapter declares, planner obeys (rationale recorded in "
            "docs/06)\n"
            "- existing tests unchanged and green; no S3 dependency (independent of "
            "#7)\n\n"
            "Refs docs/07_M3_S3_Multipart_Spike.md §2."
        ),
    },
    {
        "title": "MinIO CI harness (3 OS) + kill-9 resume drill + >100 MiB nightly e2e",
        "labels": ["ci"],
        "body": (
            "## Problem\n"
            "S3 tests need a live S3-compatible server. GitHub `services:` containers "
            "are Linux-only; using them would make the S3 gate Linux-only and weaken "
            "the 3-OS guarantee (docs/07 §6). MinIO ships a single static binary per "
            "OS.\n\n"
            "## Acceptance criteria\n"
            "- `scripts/minio-test.sh`: download pinned MinIO binary per OS "
            "(linux/darwin/windows), start, wait on `/minio/health/ready`, create "
            "bucket, export IUM_S3_* env, run env-gated S3 tests, abort leftover "
            "multipart uploads on exit\n"
            "- ci.yml: MinIO step on ALL 3 OS runners (fmt/scaffold jobs unchanged)\n"
            "- nightly: >100 MiB multipart upload + SIGKILL mid-body + restart → "
            "ListParts rebuild → assertion that bytes moved after restart < file size "
            "(zero re-upload of completed parts, wire-byte counter evidence) → "
            "complete → verify → Done\n"
            "- kill-9 drill runs on Linux in nightly (roadmap M3 DoD); the 3-OS "
            "harness is still proven for the regular S3 suite\n"
            "- honest Done only after adapter verify (ADR-010)\n\n"
            "Blocked by #7. Refs docs/07_M3_S3_Multipart_Spike.md §3, §6, §7; "
            "roadmap M3 DoD."
        ),
    },
]


def api(path, payload=None):
    token = TOKEN_PATH.read_text().strip()
    url = f"https://api.github.com/repos/{REPO}{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
        },
        method="POST" if data is not None else "GET",
    )
    try:
        with urllib.request.urlopen(req) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code}: {e.read().decode()[:300]}")
        sys.exit(1)


def main():
    # Milestone: find by title or create
    milestones = api("/milestones?state=all")
    ms = next((m for m in milestones if m["title"] == MILESTONE["title"]), None)
    if ms is None:
        ms = api("/milestones", {"title": MILESTONE["title"], "description": MILESTONE["description"]})
        print(f"created milestone '{ms['title']}' #{ms['number']}")
    else:
        print(f"milestone '{ms['title']}' already exists #{ms['number']} (open={ms['open_issues']}, closed={ms['closed_issues']})")

    # Issues: skip exact-title matches among open issues
    existing = api("/issues?state=open&per_page=100")
    titles = {i["title"] for i in existing if "pull_request" not in i}
    for spec in ISSUES:
        if spec["title"] in titles:
            print(f"skip (exists): {spec['title']}")
            continue
        created = api(
            "/issues",
            {
                "title": spec["title"],
                "body": spec["body"],
                "labels": spec["labels"],
                "milestone": ms["number"],
            },
        )
        print(f"filed #{created['number']}: {created['title']}  [{', '.join(spec['labels'])}]")


if __name__ == "__main__":
    main()
