**Filed from #33's first live rclone run** (server-diversity probe): rclone 1.75.2
`serve s3`, pinned, sha256-verified via per-release SHA256SUMS, local sandbox.

## RED evidence (current HEAD, first-ever run against a third S3 implementation)

`bash scripts/s3-test.sh --server rclone` → **22/23**; the failure:

```
test s3_zero_byte_takes_the_putobject_path ... FAILED

thread 's3_zero_byte_takes_the_putobject_path' panicked at ium-core/tests/adapter_s3.rs:594:65:
called `Result::unwrap()` on an `Err` value: Unknown("PutObject returned 411 Length Required (code=MissingContentLength)")
```

## Wire-fact analysis (gofakes3 source, rclone's fork)

`createObject` refuses a PUT that carries NEITHER `Content-Length` NOR
`Transfer-Encoding: chunked` with `411 MissingContentLength`. Our zero-byte
PutObject sends `Body::from(Vec::new())` and hyper goes out with neither —
SeaweedFS was lenient and accepted it, so the hole was invisible for two
milestones. **AWS S3 documents Content-Length as required for PutObject** —
this is not a gofakes3 quirk; the probe caught a latent real-AWS defect
through a third implementation, exactly what #33 predicted for the
adapter ↔ server-wire seam.

The multipart path is NOT affected: UploadPart already sends an explicit
`content-length` extra header (adapter_s3.rs, the `extra = [("content-length",
span.to_string())]` line).

## Fix (minimal)

The zero-byte PutObject branch in `upload_chunk` adds the explicit
`("content-length", "0")` extra header — same mechanism UploadPart already
uses. No signer change (the signer already signs extra headers; see #6's
signed-headers rule).

## Acceptance

- [ ] `s3_zero_byte_takes_the_putobject_path` green against rclone AND SeaweedFS
- [ ] full gates green; both `s3-test.sh` lanes green
- [ ] wire-fact delta recorded on #33 + docs/08 register row updated same-commit

## Refs

#33 (filing probe), #9 (SeaweedFS leniency precedent), #6 (signer header rule)
