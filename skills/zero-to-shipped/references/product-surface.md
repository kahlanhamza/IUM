# Product Surface — public without leaking

*Reference for `zero-to-shipped`. Read before making ANYTHING public.*

The pattern: **private source + public distro.** The source repo stays private forever; a separate public repository is the product's face — the thing users, search engines, and other engineers see. This file is the rules for standing that surface up and keeping it leak-free, as executed across a real day-10/11 move (releases 33→15 assets, repo built from zero, README finished to a dark-mode logo polish).

---

## 1. The split (why two repos)

- **Source repo (private):** all code, all issues, all internals, the full record. The team's everything.
- **Distro repo (public):** description, topics, product README, user guide, sanitized changelog, releases with the clean asset set, and *nothing else* — zero code, zero internals.

What this buys:
1. Users get a clean, fast, marketing-grade page without the construction scaffolding.
2. The leak surface is a *separate, small, auditable* repo — not "everything, minus careful redactions."
3. CI and quota problems on the private side never take the public downloads down (the distro's releases are independent of the source repo's visibility).
4. The updater feed lives in the distro repo → **auto-update survives any future visibility change of the source repo.** (The inverse is also the warning: feeds baked into installers before the move died the moment the source repo went private — every pre-move install needed one documented manual update to escape.)

**Publish bridge:** the source repo's release workflow publishes *into* the distro repo using one dedicated secret (scoped to that repo's contents only). The token and the secret holding it rotate together — see incident runbooks RB-3.

## 2. Repo setup (the first 30 minutes)

1. **Description:** one sentence of value, then the transport/feature keywords. Format that works: *"Product — the one-sentence promise. One window, any destination: A, B, C, D. The two killer properties."* No internals, no team words, no version history.
2. **Topics:** the ecosystem tags users search (`desktop-app`, `cross-platform`, `rust`, `tauri`, plus the domain nouns: `s3`, `webdav`, `google-drive`, `resumable-uploads`, `tus`...). ~10, specific beats generic.
3. **Files:** `README.md` (product), `USERGUIDE.md` (install → verify → connect → upload → troubleshoot), `CHANGELOG.md` (sanitized). Nothing else. Every extra file is leak surface and stale-doc debt.
4. **Sanitization pass** on anything ported from the private repo: strip issue numbers, internal codenames, team vocabulary, milestone language, internal tool names, provenance/verification-scheme internals. The public changelog says *what changed for the user*; the private one says *why and how*.

## 3. The README (big-tech structure)

Structure that survived an owner review at a real product's launch:

```
[adaptive logo]                                  ← <picture>, dark/light aware
# Product
**The one-sentence promise.**                    ← bold, standalone

The two-sentence expansion: what it does, the killer properties
("resume that actually resumes, and progress that never lies to you").

badges: platform | license | auto-update         ← shields, user-meaningful only

## Highlights                                    ← 4-6 bullets, outcome language
## Transports                                    ← table: destination | resumable | notes
## Screenshots                                   ← 2x3 grid, real harness shots, alt text
## Download                                      ← per-OS table: file | checksum | notes
## Verify your download                          ← sha256 commands per OS
## Install                                       ← per-OS incl. SmartSmart/Gatekeeper truth
## Auto-updates                                  ← how the updater works, from the user's seat
## Privacy                                       ← the stance as a feature ("no telemetry")
## Support                                       ← where issues go
license footer                                   ← proprietary statement, honest terms
```

The two hard rules:
- **What it does / how to use it — never how it's made.** No architecture, no stack decisions, no engineering process, no benchmarks you can't defend publicly, no issue references.
- **Everything a user can act on.** Every section either answers a question a buyer has or gives them a button to press. Nothing that only the team cares about.

## 4. Assets (the details that signal quality)

- **Logo: dark/light adaptive.** GitHub's dark theme is the default for your audience. Ship both inks and switch on `prefers-color-scheme`:
  ```html
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-white.svg" />
    <img src="assets/logo.svg" width="96" alt="The logomark" />
  </picture>
  ```
  The failure it fixes is exactly the kind users report first: *"the logo is dark and GitHub readme is dark so it's not visible."* Keep both files in the distro repo (not hotlinked from the private repo — unauth users get 404, and the URL itself leaks the private repo's existence). Rasterize-verify both inks on both surfaces (`#0D1117` canvas, white canvas) before shipping.
- **Screenshots through a real harness.** Six shots from the real app against real destinations — never mockups. Every image gets alt text. The set covers: main window, the filled form, the detail view, settings, credentials, and the light scheme (which doubles as the light-mode proof).
- **Checksums in the download table**, per file, matching the published `SHA256SUMS.txt` exactly (same names, same bytes — filename drift 404s the verify step).
- **Honest installer warnings:** unsigned-binary reality (SmartScreen/Gatekeeper) documented with the actual commands, not reassurance.

## 5. The source-zip awareness (the sneakiest surface)

Every GitHub release ships auto-generated **"Source code (zip/tar.gz)"** links. They archive **the release repo's tree at the tag** — so:
- the zip contains exactly what the distro repo contains at that tag (its README, assets...). If the distro repo is product-only, the zip is product-only. This is a *feature*: keep the repo clean and the zip is clean.
- the zip's README is the tree **at the tag**, not at `main`. A README improvement on main after the release does NOT reach the zip until the tag moves (moving a published tag = incident runbook RB-4 territory). If the surface changed materially, either move the tag with full re-verification or accept the zip lag and record it.
- "This release contains the source code!" is a classic false alarm (RB-5) — download and list the zip before reacting, then fix whatever real gap it exposed.

## 6. The leak gate (one-way rule)

Every asset that goes public passes an **allowlist**, checked by a gate that hard-fails otherwise:

```
ALLOWED in a public release:
  installers/bundles + per-installer .sig sidecars
  SHA256SUMS.txt (computed over the public files only)
  latest.json (signed manifest, user-facing fields only)

NEVER:
  SBOMs (CycloneDX/SPDX)         — embed the private dependency closure
  attestations (cosign/DSSE)     — embed private repo URL, workflow path, runner image, private SHA
  anything citing internal issues/ADRs/team vocabulary
  any URL pointing at the private repo
```

The gate generates provenance artifacts into a **private-only** directory riding CI artifacts, and its self-test (RED fixture + clean fixture) rides CI. This gate is the difference between "a release" and "a source leak dressed as a release" — the real-world audit found 18 such files riding a glob into a public release.

**The user-view leak check:** after any surface change, do the whole tour *unauthenticated*: fresh browser/incognito mindset — repo page, README, each release, each asset URL, the source zip contents, every image URL (raw fetch → 200 or 404). Anything that 404s unauth but looks fine to you is hotlinked from the private repo — fix it. Anything that 200s but contains internal vocabulary is a leak — fix it. The tour takes ten minutes and it is the only test that runs with the attacker's (and the user's) privileges: none.

## 7. The day-11 polish lesson

The surface is never "done" on the day it launches. The dark-logo report came *after* the launch, from the owner viewing the page in dark mode — the single highest-traffic view. Fix, re-verify both themes, re-check the zip (it ships the README), record the lesson. Budget for one polish pass on any public surface, and treat every report about it as a gift: the owner reading their own product page in the dark found in one look what ten authed reviews missed.
