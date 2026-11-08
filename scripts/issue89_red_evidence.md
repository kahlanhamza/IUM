# RED evidence — #89 (nightly #12 drill bootstrap)

Repro: `scripts/repro_issue89_red.sh` — a local HTTP server that answers every
request `504` with an HTML body (mimicking the CDN brown-out of run 42468487548),
then runs the fetch+guard lines **verbatim** from `scripts/tus-test.sh` L157–163 at HEAD.

```
=== HEAD fetch lines (verbatim from scripts/tus-test.sh L157–163) ===
[tus-test] FAIL: sha256 mismatch for fake_artifact.tar.gz (expected <html><body><h1>504, got f9eeae29260f783ddadf5c748b1a43c9439896cf689b0c9fac9f0bf87965468c) — supply-chain guard

=== Forensics ===
sidecar bytes: <html><body><h1>504 Gateway Time-out</h1>The server didn't r
curl exit codes: tarball=0 (last op) — both curls above exited 0 or the DIE lines would have printed
```

Facts demonstrated at HEAD:

1. **curl exits 0** on the 504 — `|| die "download …"` never fires (no `--fail`).
2. **The HTML error page is written into the `.sha256` sidecar** (60 bytes shown above).
3. The supply-chain guard then surfaces the **cryptic mismatch** `expected <html><body><h1>504`
   — byte-identical in shape to the nightly #12 failure (`tus_drill` job 128693457248,
   `s3_drill` job 128693457355). The guard held; the error sent triage toward a checksum
   regression instead of a network blip.

RED on the record. Fix follows in the same lane: `-f` + `--retry-delay 2` on the eight
artifact/sidecar fetches in `tus-test.sh`/`s3-test.sh`, plus a sidecar digest-format guard
so a 200-with-error-page also dies naming the true cause.
