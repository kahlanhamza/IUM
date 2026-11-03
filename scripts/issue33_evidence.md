## Evidence — both lanes ran live, one real defect found and fixed (#36)

### Lane 1: third S3 implementation — rclone 1.75.2 `serve s3` (pinned, SHA256SUMS-guarded)

**First-ever run against a third implementation (RED, at the pre-fix HEAD): 22/23.**

```
test s3_zero_byte_takes_the_putobject_path ... FAILED
called `Result::unwrap()` on an `Err` value:
  Unknown("PutObject returned 411 Length Required (code=MissingContentLength)")
test result: FAILED. 22 passed; 1 failed
```

The failure is a REAL adapter defect, not a server quirk: hyper sends neither
`Content-Length` nor chunked-TE for an empty fixed-size body, and AWS S3
semantics — which gofakes3 (rclone's serve-s3 engine) enforces with 411 where
SeaweedFS happened to be lenient — require a length on PutObject. Filed as
**#36** with this RED; fixed in `fix(core)` the same way UploadPart already
did it (explicit `content-length: 0`). **After the fix: rclone 23/23 GREEN,
SeaweedFS (canonical lane re-run) 23/23 GREEN.**

**Wire-fact ledger for the third implementation (each pinned or recorded):**

| Wire fact | rclone/gofakes3 1.75.2 | Verdict |
|---|---|---|
| ListParts | routed and answered (`GET ?uploadId=` → `ListPartsResult`) although rclone's own docs OMIT it from the supported-operations list | recorded — doc-vs-implementation delta; suite's ListParts tests pass unmodified |
| ETag entity encoding | Go `encoding/xml` renders `"` as `&#34;` — NUMERIC character references in ListParts ETags, the exact #9 quirk class | pinned — existing `xml_text` numeric-decoder tests + the live suite passing through real rclone ETags |
| `ListPartsResult` shape | Part/PartNumber/Size/ETag, IsTruncated, NextPartNumberMarker (int) | pinned — `parse_list_parts` unmodified |
| NoSuchUpload on unknown/completed uploadId | 404 | pinned — `s3_bogus_session_maps_to_expired_session` passes unmodified |
| PutObject without length | **411 MissingContentLength** (AWS-real; SeaweedFS lenient) | **#36 — fixed** |
| unsigned `x-amz-*` headers | refused (AccessDenied) | pinned — #6's signer signs everything it sends; suite green |
| SigV4 / path-style / CreateBucket / Complete | compatible | pinned — full lifecycle, restart-drill, verify-refuses tests all pass unmodified |

### Lane 2: second TUS implementation — `@tus/server` 2.4.7 + `@tus/file-store` 2.1.2 (npm-pinned)

**First run: 12/12 GREEN — with ONE loud, recorded skip (the expected delta).**

```
test result: ok. 12 passed; 0 failed
[tus-test] GREEN: TUS tests passed against tus-node on Linux/x86_64
```

**Wire-fact ledger (probed live, then confirmed by the suite):**

| Wire fact | @tus/server 2.4.7 | Verdict |
|---|---|---|
| GET-download extension | **404 on the upload resource** — tusd's GET is its own extension, NOT tus 1.0.0 core | recorded + pinned skip — `tus_full_lifecycle_two_parts_and_readback` (and the real-data drill's TUS leg) skip LOUDLY when `IUM_TUS_SERVER=tus-node`; every other test runs |
| POST creation | 201 + absolute Location (observed `…/files//<id>` — double slash when the base path ends in `/`; requests on that URL work) | recorded; adapter's RFC 3986 §5.2 resolver takes the absolute URL verbatim — suite green |
| PATCH wrong offset | **409** (spec) | pinned — `tus_offset_conflict_realigns_from_server_offset` passes unmodified |
| PATCH without `Tus-Resumable` | **412** (spec) | consistent with #10's pinned behavior |
| PATCH/HEAD acks | 204 + `Upload-Offset`, HEAD 200 + length/offset | pinned — partial-confirm + zero-reupload tests pass |
| HEAD on unknown id | 404 → `SessionLost` | pinned — `tus_bogus_session_maps_to_session_lost` passes unmodified |
| OPTIONS | **204** (tusd answers 200) — both carry `Tus-Resumable: 1.0.0` | recorded — harness readiness check accepts both |

### Ecosystem verdict on the third TUS lane question

A second independent TUS 1.0.0 server EXISTS and is pinnable: tusd (Go, the
reference) + `@tus/server` (Node/TypeScript, actively maintained) + tusdotnet
(.NET). Both CI lanes now run the official Go reference AND the Node
implementation. The optional owner-input AWS/B2 smoke lane remains marked as
such (needs owner creds, never a CI dependency) — same offer as #32's cloud legs.

### Acceptance criteria checklist

- [x] third S3 lane: `s3-test.sh --server rclone`, pinned 1.75.2, checksum-verified (SHA256SUMS — rclone ships no per-asset md5; recorded as harness fact), suite green (23/23)
- [x] second TUS lane: `tus-test.sh --server tus-node`, pinned via committed package-lock.json, suite green (12/12 incl. 1 loud recorded skip)
- [x] owner-input lane: recorded as available-if-creds, never a CI dependency
- [x] adapter change required by a delta → its own RED + own issue: **#36** (fixed this session, both lanes green after)
- [x] register rows (adapter ↔ server wire ×2, harness ↔ reality) updated same-commit
- [x] CI: Linux diversity legs wired in ci.yml; canonical SeaweedFS/tusd legs unchanged on all 3 OS
