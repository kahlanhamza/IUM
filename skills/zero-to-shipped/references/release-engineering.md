# Release Engineering — the full bar

*Reference for `zero-to-shipped`. Read before cutting ANY release.*

This playbook shipped 7 tags in 11 days (v0.1.0, v0.1.1-rc.1, v0.1.1, v0.1.1-1, v0.2.0, v0.2.1, v0.2.2) across three platforms with auto-update, signed manifests, and a public distribution surface — and every asset byte-verified. It is organized as the ordered checklist you run every time.

---

## Phase A — the version sweep

Move every manifest together. The sweep exists because a stray manifest **did** lag twice (a shell crate left at 0.1.1 while the product moved to 0.2.1 — caught by the sweep's grep, not by luck):

```bash
# every file that carries a version must carry THE version
grep -rn "0\.2\.0" --include="*.toml" --include="*.json" --include="*.lock" . \
  | grep -v target | grep -v node_modules
```

- workspace `Cargo.toml` + every member crate's `Cargo.toml` (the shell crate is the classic straggler),
- `package.json` + lockfile,
- the app manifest (`tauri.conf.json` — version *and* the updater endpoint + signing pubkey while you're in there),
- `Cargo.lock` regenerated, not hand-edited.

**Semver honesty:** version by content. In the source project, a release carrying only a security fix + docs arc shipped as a *patch* and the CHANGELOG said so. Users can smell version inflation; the record keeps you honest.

**CHANGELOG:** written for the user, sanitized for any public surface (no internal issue numbers on the public copy — see `product-surface.md`).

---

## Phase B — CI green, then tag (ADR-007: no tag until CI green)

1. All gates green on the committed tree.
2. Push; chase the CI verdict to completion (loop state 7).
3. **Only then** cut the annotated tag:
   ```bash
   git tag -a v0.2.2 -m "v0.2.2 — the distro move: the updater feed + the downloads ship from the public distro"
   git push origin v0.2.2
   ```
   The tag message is a one-line release note — it shows up in the archive page and in `git log --tags`.
4. The tag triggers the release workflow. Poll it like any run. A release workflow is *bigger* than CI (three OS build legs + a publish leg); budget wall-clock accordingly.

**Never tag on:**
- a red or unknown matrix,
- an unverified "docs-only" exception,
- a tree you haven't pulled (tagging a stale local HEAD ships yesterday's code with today's version number).

---

## Phase C — the publish leg (asset set + provenance hygiene)

### The allowlist is the writer

The public asset set is defined by an explicit allowlist, **not a glob**. The source project's original glob (`release_assets/*`) shipped 33 assets including 18 provenance sidecars; the fix made the publish step enumerate exactly:

```
installers/bundles per platform   (the real shippables)
└── .sig sidecar for EVERY installer   (the updater verifies these)
SHA256SUMS.txt                    (checksums over the public files ONLY)
latest.json                       (the signed updater manifest)
```

and **hard-fails** if any provenance-shaped file appears in the upload set:

- `*.sbom`, `*.cdx`, CycloneDX/SPDX anything,
- cosign/attestation bundles (DSSE envelopes),
- anything referencing the private source repo, workflow paths, runner images, or internal commit SHAs.

SBOMs and attestations are **valuable internal artifacts** — generate them, but into a private-only directory that rides the CI artifact store, never the public release. They embed: private repo URLs, workflow file paths, runner images, the private commit SHA. Publishing them is publishing your internals.

**Why allowlist and not denylist:** a denylist leaks by default (new artifact type = silently public). An allowlist is closed by default. The gate script (`--self-test` with RED and clean fixtures, riding CI) is what keeps this from decaying.

### Cross-repo publish

When source is private and distribution is public, the publish job pushes to the public distro repo with a dedicated secret scoped to that purpose (`DISTRO_TOKEN` — `contents:write` on the distro repo only, nothing on the source repo). Coupling rules:
- the token and the repo secret holding it **rotate together** (they must be the same value at publish time),
- the publish leg runs the hygiene gate *before* uploading (detector before actor),
- release notes are generated user-facing: checksums, silent-install flags, the updater explanation — zero internal references.

---

## Phase D — the unauth verification battery (verify-before-declare, for releases)

Run it as the user: **no credentials, plain HTTPS, real downloads.** This battery exists because it *caught real bugs* — the platform-key mismatch (#100) would have broken the updater for every existing install.

1. **The feed resolves:**
   ```bash
   curl -sL https://<distro>/releases/latest/download/latest.json
   # 200; version == the tag; platform keys == hardware you actually ship
   # (darwin-aarch64 / linux-x86_64 / windows-x86_64 — not darwin-x86_64 because
   #  a v1-era script guessed, #100's whole lesson)
   ```
2. **Signatures byte-match:** each platform entry's `signature` field == the published `.sig` asset's bytes, exactly (decode both base64 layers if needed).
3. **Checksums match on real artifacts:** download at least one installer per platform (the deb, the AppImage, the dmg, the setup.exe), `sha256sum` them, compare against the published `SHA256SUMS.txt`. Byte-exact or the release is not shipped.
4. **Latest is latest:** the release is the repo's `latest`, not draft, not prerelease.
5. **The source zip is current:** download the auto-generated "Source code (zip)" and inspect it — it archives the *release repo's tree at the tag*, so confirm the README inside is the current product README (a stale tag ships a stale README into every zip — caught live on day 11).

Post the battery results as the GREEN evidence on the release issue. **The release is shipped when the battery says so — not when the workflow turns green.** (A workflow can succeed and still have published the wrong thing; only bytes prove shipping.)

---

## Phase E — tag choreography (the minefield)

The rules, each earned:

1. **Never delete the tag of a published release.** GitHub flips the release to **draft** when its tag ref is deleted → the release vanishes from `/releases/latest` → **every user's updater feed 404s**. This was hit live (day 11, ~2 minutes of dark feed) and recovered via:
   ```
   PATCH /repos/<owner>/<repo>/releases/<id>   {"draft": false, "tag_name": "v0.2.2"}
   ```
2. **Recreating the ref does not re-publish.** Recreate the tag if you must (same name), then PATCH the release back out of draft, then re-run the unauth battery. Expect the archive to lag (~a minute of CDN cache on the source zip).
3. **Moving a tag = moving the source archive.** The release's assets are untouched by a tag move, but the auto source zip regenerates from the tag — if you moved the tag to pick up a README fix, verify the zip actually picked it up (attempt-loop with sleeps; CDN cache).
4. **The updater's trust root is the signing key, not the repo.** The feed URL can move (private → distro), the tag can move; the pubkey pinned in the app is the thing that must never rotate casually. Rotating it is a *product* release, not a repo operation.
5. **v(N-1) must survive.** The upgrade-path lane (#96) proved a mid-flight v0.2.1 journal resumes byte-exact under v0.2.2 — users don't upgrade on your schedule; they upgrade mid-upload, in the rain, at 2%. The release bar includes proving the N-1 path, not hoping.

---

## The release-day runbook (condensed)

```
[ ] version sweep (every manifest = THE version; lock regenerated)
[ ] CHANGELOG written, honest semver, public copy sanitized
[ ] gates green locally on the committed tree
[ ] push → CI verdict chased → GREEN with run ID on record
[ ] annotated tag cut + pushed
[ ] release workflow polled to SUCCESS (all build legs + publish leg)
[ ] publish leg's hygiene gate ran BEFORE upload (allowlist, zero provenance)
[ ] unauth battery: feed 200 + version + platform keys = shipped hardware
[ ] unauth battery: manifest sig == .sig asset bytes (every platform)
[ ] unauth battery: >=1 real installer per platform, sha256 == SHA256SUMS
[ ] release is latest / not draft / not prerelease
[ ] source zip inspected (current README, no internals)
[ ] GREEN evidence posted; release issue closed
[ ] STATUS row appended; record pushed
```

A release that skips a checkbox isn't a faster release — it's an unverified one. The two times the source project's batteries caught shipped-feed breakage (#100's platform keys, day 11's draft flip) each cost less than ten minutes to catch and would have cost every user their auto-update to miss.
