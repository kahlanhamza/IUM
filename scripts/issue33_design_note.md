## Design note — server diversity probe (read before implementation)

**Method**: same as #32's probe — the FIRST live run against each new server IS the RED
evidence. The suites are env-gated; at current HEAD no third-S3 / second-TUS lane exists
at all. Every failure the first runs produce becomes either a pinned wire-fact test or a
loud, recorded honest skip. Nothing silently absorbed.

### Lane 1 — third S3: `rclone serve s3` v1.75.2 (pinned)

- Harness: `scripts/s3-test.sh --server rclone` (default stays `seaweedfs`; every existing
  caller and CI leg unchanged).
- Asset: `downloads.rclone.org/v1.75.2/rclone-v1.75.2-{os}-{arch}.zip`. rclone publishes NO
  per-asset md5 sidecars — the supply-chain guard is the per-release `SHA256SUMS` file,
  grepped for the asset line. Same guarantee, different shape (recorded as harness fact).
- Boot: `rclone serve s3 --auth-key iumcikey,<secret> --addr 127.0.0.1:$PORT <tmpdir>`
  — buckets = root directories, path-style by default (`--force-path-style` default true).
- Static pre-analysis from rclone's gofakes3 fork source (to be CONFIRMED live, not trusted):
  1. **ListParts IS routed** (`GET /bucket/key?uploadId=` → `listMultipartUploadParts`,
     routing.go:185) even though rclone's own docs omit it from the supported-operations
     list → doc-vs-implementation delta to record.
  2. **ETag entity encoding**: Go `encoding/xml` renders `"` as `&#34;` in element text →
     ListParts ETags arrive as NUMERIC character references — the exact quirk class #9
     fixed for SeaweedFS. `xml_text`'s single-pass decoder should absorb it; the live run
     pins this with a real assertion, not an assumption.
  3. `ListPartsResult` field set matches `parse_list_parts` (Part/PartNumber/Size/ETag,
     IsTruncated, NextPartNumberMarker — int-typed in gofakes3).
  4. Unknown/completed `uploadId` → `ErrNoSuchUpload` → 404 → our already-finalized skip
     path (adapter_s3 verify step 1) must hold.
  5. Every `x-amz-*` header must be in the signed-headers set (rclone serve s3 refuses
     unsigned `x-amz-*` with AccessDenied) — #6's signer already signs all sent headers.
- Anything the run breaks → per-test honest skip WITH the delta recorded here, or a real
  adapter defect → its own issue with RED (the issue's rule).

### Lane 2 — second TUS: `@tus/server` 2.4.7 + `@tus/file-store` 2.1.2 (pinned)

- Pinned via committed `package.json` + `npm ci` (lockfile integrity hashes = the
  supply-chain guard; node 20 in CI, node 24 locally).
- Tiny server script `scripts/tus-node-lane/server.mjs`: path `/files/`, FileStore on a
  temp dir, core + creation extension only. Harness: `scripts/tus-test.sh --server tus-node`.
- Expected wire-fact delta (to confirm live): **tusd's GET-download extension does not
  exist on @tus/server** (tus 1.0.0 core has no download) → the readback test
  (`tus_full_lifecycle_two_parts_and_readback`) becomes an HONEST skip on this lane,
  driven by a new `IUM_TUS_SERVER` env the harness exports; the delta recorded here.
- 409 on offset mismatch and 412 without `Tus-Resumable` are spec-mandated — expected to
  hold; the run verifies.

### Pinning discipline & register

- Every live delta → pinned test (server named in the test/doc comment) or loud recorded
  skip on this issue. Never silent.
- `docs/08` boundary register rows (adapter_s3 ↔ server wire, adapter_tus ↔ server wire)
  gain the second/third server evidence in the SAME commit.
- CI: `--server rclone` and `--server tus-node` legs on ubuntu-latest (both free); the
  pinned SeaweedFS/tusd legs stay canonical on all 3 OS.
- No adapter changes are expected — if one turns out to be required, it goes through its
  own RED + own issue per this issue's acceptance criteria.
