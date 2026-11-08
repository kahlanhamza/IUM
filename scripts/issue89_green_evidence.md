# GREEN evidence — #89 (drill bootstrap fails loud)

Fix on `a1589cc`: `-fsSL --retry 3 --retry-delay 2` on all six artifact/sidecar
fetches in `scripts/tus-test.sh` + `scripts/s3-test.sh`, plus digest-format
guards (64-hex sha256 / 32-hex md5 / SHA256SUMS line) before every compare.

Repro: `scripts/repro_issue89_green.sh` — the verbatim hardened fetch lines
against three adversarial servers:

```
=== Case A ===            (504 + HTML for everything — the nightly #12 brown-out)
curl: (22) The requested URL returned error: 504     ← attempt 1
curl: (22) The requested URL returned error: 504     ← retry 1
curl: (22) The requested URL returned error: 504     ← retry 2
curl: (22) The requested URL returned error: 504     ← retry 3
[tus-test] FAIL: download artifact.tar.gz — upstream error (HTTP 4xx/5xx?)
              (no file written — the guard's sidecar can no longer be poisoned)

=== Case B ===            (200 + HTML body in the sidecar — captive portal case)
[tus-test] FAIL: sha256 sidecar for artifact.tar.gz is not a 64-hex digest — upstream served an error page? (supply-chain guard)

=== Case C ===            (healthy tarball + true sidecar)
[tus-test] downloaded + sha256 verified

bash -n both scripts: tus-test.sh: syntax OK / s3-test.sh: syntax OK
```

## Nightly rerun — the flake confirmed environmental

Rerun of the failed jobs of nightly run 42468487548 (same commit 703e991,
attempt 2 — REST `/actions/runs/42468487548`):

```
nightly attempt 2 | completed / success
  tus_drill  success    s3_drill  success
  coverage   success    fuzz      success
```

Same commit, healthy upstream → all drills pass. The engine was never broken;
the bootstrap was. Attempt 1's failure mode (504 → poisoned sidecar → cryptic
mismatch) is now structurally impossible: HTTP errors fail the fetch loud after
1+3 retries with backoff, and a non-digest sidecar dies naming the true cause.

## Gates on a1589cc

- `gate.sh` GREEN on the committed tree (core 313/313, vitest 129/129 across
  16 files, tsc strict, vite build; WebKitGTK rust lane → CI as always).
- CI run 42486781964 on `a1589cc`: **8/8 success**.
- Follow-up sweep filed as #90 (same hardening for the remaining fetch sites:
  issue50/55 rehearsals, release.yml, stress.yml, dbg-s3-iso.sh, release_supply_chain_check.sh).
