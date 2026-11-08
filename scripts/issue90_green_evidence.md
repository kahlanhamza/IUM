# GREEN evidence — #90 (the curl sweep)

Commit `d349ef8` — the #89 defect class (`curl` without `-f` saves upstream
error pages with exit 0) swept across the remaining six fetch sites:

| Site | Change |
|---|---|
| `scripts/issue50_wire_rehearsal.sh` | `-f --retry 3 --retry-delay 2` on both fetches + 64-hex sidecar guard |
| `scripts/issue55_wire_rehearsal.sh` | same (cached variant) |
| `.github/workflows/stress.yml` | `-f` on both film fetches; the 30 MB clip gains its **real pinned sha256** (`5b603ec1db36cc15d5e48ab9ea6e72ffd6a65e325f2bb218e9ed298974b453bc`, fetched live, 30 704 510 B) replacing the weak ≥10 MB size floor |
| `.github/workflows/release.yml` | `-f --retry 3 --retry-delay 2` on the pinned syft + checksums fetches |
| `scripts/dbg-s3-iso.sh` | `-f --retry-delay 2` + loud failure |
| `scripts/release_supply_chain_check.sh` | `-f` on the attested-asset fetch — an error page can no longer masquerade as "NO build provenance attestation" |

Verification:

- `bash -n` clean on all four scripts; both workflows YAML-parse (python yaml.safe_load).
- Zero engine surface — `gate.sh` GREEN on the committed tree `d349ef8`
  (core 313/313, vitest 129/129 across 16 files, tsc strict, vite build,
  rustdoc `-D warnings`; WebKitGTK lane → CI).
- The stress.yml film pins keep the S1 size proof (322 866 076 B) intact.

Sandbox ops note (STATUS M7/M13#5 lesson re-applied): the toolchain
reinstall lost `.toolchain/cargo/config.toml` (the GNU-ld pin) — recreated
gitignored; a stale 7.8 GB target cache from the pre-pin builds purged via
`cargo clean` + `CARGO_INCREMENTAL=0` (the 2026-11-05 disk discipline).
