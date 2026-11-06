# Lane 4 (#66): SBOM + provenance in release.yml

Product bar #66, lane 4: **supply-chain integrity** — every release carries a
machine-readable SBOM and verifiable build provenance. Code signing stays the
pinned owner ask (#65); this lane closes the *other* half of "what big teams
actually ship".

## Wire facts (all executed in this sandbox today, none assumed)

- **W1** — `release.yml` today: tag-triggered 3-OS installer matrix (#65 Lane 2)
  + smoke installs + publish job; top-level `permissions: contents: write`;
  all 7 action refs are **tag-pinned** (mutable — a hijacked tag silently
  changes what builds the artifacts we sign).
- **W2** — the only tag, `v0.1.0`, predates the installer pipeline: its release
  has **zero assets**. No release in this repo's history carries an SBOM or an
  attestation. (This is the RED condition, provable at current HEAD.)
- **W3** — root `Cargo.lock` covers the whole workspace (ium-core,
  ium-app/src-tauri, ium-e2e). Rehearsed locally: syft **v1.57.0**
  (download + `sha256sum -c` OK) over `Cargo.lock` → **483 components**
  (479 cargo), tokio/reqwest/rusqlite present. CycloneDX 1.7 JSON.
- **W4** — `npm sbom --sbom-format cyclonedx` in `ium-app/` → **174 components
  including devDependencies** (vite, typescript, axe-core present; requires the
  installed tree — `ESBOMPROBLEMS` without node_modules, so the publish job
  must `npm ci` first; runner node 20 ships npm ≥10.8 ≥ the 10.1 that added
  `npm sbom`).
- **W5** — syft alone is NOT honest for the app tree: it catalogs only **25**
  npm components vs 225 `dev: true` lockfile entries — yet devDeps (vite,
  tailwind tooling, axe) are what get bundled into the shipped JS. Hence the
  per-ecosystem split below.
- **W6** — `actions/attest-build-provenance@v4.2.2` (SHA-pinned below):
  takes `subject-path` globs (≤1024 subjects), needs job perms
  `id-token: write` + `attestations: write`, works on this public repo's OIDC.
- **W7** — attestation verification REST API
  `GET /repos/{owner}/{repo}/attestations/{sha256_digest}`: unauthenticated →
  403; authenticated + none → 404; authenticated + present →
  `{"attestations":[{bundle}]}`. `gh` is absent in the sandbox but preinstalled
  on runners → in-CI self-verify uses `gh attestation verify`, post-push
  verification from the sandbox uses the REST API.
- **W8** — SHA pins resolved via API today:
  checkout `11d5960a` (v4) · setup-node `49933ea5` (v4) · upload-artifact
  `ea165f8d` (v4) · download-artifact `d3f86a10` (v4) ·
  attest-build-provenance `4d101475` (v4.2.2) · dtolnay/rust-toolchain
  `b725eda5` (branch `stable`) · Swatinem/rust-cache `d1527c8f` (v2).
- **W9** — version lives in 4 places (root `Cargo.toml` workspace.package,
  `ium-app/src-tauri/Cargo.toml`, `tauri.conf.json`, `ium-app/package.json`),
  all `0.1.0` → the rehearsal release bumps all four to `0.1.1-rc.1`.
- **W10** — the publish job checks out the **tag ref**, so SBOMs generated
  there describe exactly the tagged tree.

## Design

**Two SBOM assets per release (per-ecosystem, both CycloneDX JSON):**

1. `IUM-<tag>-rust.cdx.json` — syft **v1.57.0, downloaded with checksum
   verification in-logs** — over the root `Cargo.lock` (the whole engine +
   Tauri backend closure). No config, no excludes needed at file scope.
2. `IUM-<tag>-app.cdx.json` — `npm sbom` (npm-native CycloneDX) over
   `ium-app/package-lock.json` **including devDependencies** (W5: they ship).

One merged "everything" SBOM was rejected: W5 shows the merge tool drops what
actually ships; per-ecosystem files match how each ecosystem's tooling is
audited and verified.

**Provenance:** `actions/attest-build-provenance` in the publish job on every
shipped artifact (installers + both SBOMs) — one attestation store per repo,
SLSA v1 provenance, verified with `gh attestation verify -R kahlanhamza/IUM`.
The sigstore bundles are ALSO downloaded (`gh attestation download`) and
attached as release assets, so verification survives the repo going private
(#66: public is temporary) — bundles verify offline with cosign against the
Sigstore Fulcio/Rekor transparency logs.

**Workflow hardening (same lane, same file):** all 8 actions SHA-pinned (W8,
version kept in a trailing comment); least-privilege permissions — top-level
`contents: read`, publish job exactly `contents: write, id-token: write,
attestations: write`.

**Publish-job order:** download artifacts → assemble installers → node +
`npm ci` → syft (checksummed) → two SBOMs → **attest all of it** →
`gh attestation download` bundles → `gh attestation verify` **in-job (the
gate — release cannot ship if verification fails)** → SHA256SUMS over
everything → release notes gain a "## Supply chain" section → publish
(`--prerelease` when the tag contains `-`, per semver).

## RED (before any workflow change)

`scripts/release_supply_chain_check.sh <tag>` — pure REST (no `gh`, works from
the sandbox), asserts per tag: release exists; ≥ 2 `*.cdx.json` assets; each
parses as CycloneDX with ≥ 100 components; **every installer asset's sha256
resolves to an attestation** via W7's API. Run against `v0.1.0` at current
HEAD → must FAIL on the missing SBOM assets (W2). Failing output pasted here
before implementation.

## Acceptance

- [ ] RED output on this issue before the fix lands.
- [ ] Gates green: cargo fmt/clippy/test (core untouched but gates are
      non-bypassable) + actionlint on release.yml.
- [ ] `feat(release): … (Closes #71)` pushed; `ci.yml` green on the push.
- [ ] **End-to-end rehearsal:** version bumped to `0.1.1-rc.1`, tag pushed,
      the real release pipeline produces installers + SBOMs + attestations
      with in-CI verification green.
- [ ] `scripts/release_supply_chain_check.sh v0.1.1-rc.1` → GREEN (evidence).
- [ ] STATUS.md row with the run + evidence links.
