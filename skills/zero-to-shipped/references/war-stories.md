# War Stories — eleven days, case by case

*Reference for `zero-to-shipped`. Every rule in this skill is here because something happened. Read before skipping rules; read when teaching the method.*

Fidelity note: these are the recorded lanes of the source project (a real desktop upload engine, private repo, one human owner + one AI agent), drawn from its 158-row session log. Names of tools and issue numbers are kept where they teach; the patterns generalize.

---

## Day 0 — the empty repo (2026-11-01)

**What happened:** The first commits were not code. Commit one: README. Commit two: an architecture teardown of five competing upload tools (aria2, XDM, Motrix, Free Download Manager 6.35, IDM 6.43) — features, limits, protocols, the gaps between them — *as clean-room evidence*. Commit three: the MVP spec with milestones M0–M9 and acceptance criteria per milestone.

**Why it matters:** by day 11, nobody argued about what "done" meant for any feature, because the acceptance criteria pre-dated the code. The teardown also seeded the differentiators (resume that actually resumes, progress that never lies) that became the product's marketing language on the public surface.

**Rule it proves:** documents before code (SKILL.md §7). The teardown brief and RE-resistance checklist (docs 07/12) later doubled as the clean-room policy's audit trail (ADR-009).

---

## Days 1–4 — the engine bar (M0–M5)

**What happened:** the core lanes ratified the load-bearing ADRs *before* implementation: journal as single source of truth (SQLite/WAL, ADR-002), dynamic chunk pool with leases (ADR-003), verify-before-declare (ADR-004), slow-connection eviction by throughput strikes (ADR-005), and the ordering decision itself — **engine first, UI frozen** (ADR-006).

**The pivot that wasn't silent:** when a decision needed amending (the UI-freeze directive), it happened as a recorded amendment on an issue — not as drift. The rule survived; the scope changed; the record shows both.

**Rule it proves:** architecture decisions are ratified with reasoning preserved, and amendments are recorded decisions too.

---

## Day 5 — v0.1.0, the first cut (#58)

**What happened:** the M9 rungs closed — S3-shaped link prober (#57), connection bonding, FEC, zstd lanes — and the first tag shipped. S3 verify() became the commit gate (ADR-010): the engine finalizes only after verifying E-tag against the sent bytes.

**Rule it proves:** "adapter declares, planner obeys" (ADR-011) was born from this lane set — transports declare their capabilities, the planner never assumes. Every later transport (day 7–8's YouTube, Drive, MEGA, TeraBox lanes) plugged in without touching the core's assumptions.

---

## Day 6 — the product exists (#65 lane set)

**What happened in one day:** the shipped-product UI pass; the per-OS installer pipeline; the credentials vault as a product surface; the design system as primitives **plus the axe a11y gate** (accessibility became a gate, not an intention); the error-copy system; a **100-jobs load e2e**; the **installer-size budget gate**; and the brand sheet (#73).

**The CI repair lesson (#61):** the shell's compile gate *lived only in CI* — meaning local runs said green while CI said red, wasting a cycle. Fix: the gate moved local, where gates belong. Gates you don't run locally are CI roulette.

**Rule it proves:** quality properties (a11y, size, load) become gates the day they're defined — and gates run in *both* places.

---

## Days 7–8 — transports and identity (M12–M13)

**What happened:** the transport lanes closed one by one (YouTube #60, Google Drive #59, the MEGA bridge seam #61, the TeraBox native lane #64 — note the *two architectures* chosen per transport, bridge vs native, each recorded); the visual identity landed as two inks + one type system + the logo ladder; the logomark — **the dart: two concentric arcs on one bow, zero pointed vertices** — was adopted through an owner-approved set process (#79); ⌘K command palette; Windows Mica material pass; a curl hardening sweep; and **v0.2.0 shipped with branch protection enabled**.

**The PAT-return story (#86/#87):** a credential-lane closed with file → prove → push → verdict as four distinct recorded steps — the loop even for security chores.

**Rule it proves:** design system + gates (the 96px identity moment, the palette ladder) went through the same issue-first loop as the engine. "It's just design" is how quality regresses silently; here it was rungs and gates like everything else.

---

## Day 9 — the killer found under stress (#95)

**What happened:** the stress `long_soak` lane found a real killer: the **wrapper amputation** bug — found *only because* a resource-hygiene bar (idle-CPU, fd leaks, sanitizer runs) existed as a lane with teeth. Fixed and re-soaked.

**Rule it proves:** you don't find soak bugs by hoping; you find them by building a soak gate and running it on a schedule. The bar (#95) came *before* the bug it caught — write that down.

---

## Day 10, morning — the release bar closes (#96/#97/#98)

**What happened, lane by lane:**
- **#96 the upgrade-path bar:** a v(N-1) mid-flight journal must resume byte-exact under the new version. Proven, not promised.
- **#97 the dependency-audit lane:** nightly cargo-audit, pinned, detector-proven — CVE response became a scheduled detector instead of a news reaction.
- **#98 the OS life-event lane:** SIGSTOP/SIGCONT freeze mid-body — counted transactions survive system sleep/stop. The OS is adversarial; the journal absorbs it.

Then the docs arc (#99: the user guide with per-OS sha256 verification, the real SmartScreen/Gatekeeper truth, the four-field form, the journal's failure receipts in user words) and the closed-product README rewrite — with a receipts gate green unmodified.

## Day 10, afternoon — v0.2.1 (#58's discipline, matured)

**What happened:** version sweep caught **the shell crate still at 0.1.1** (the manifest straggler — this is why the sweep exists); honest semver (security fix + docs = *patch*); CI 13/13 before the tag; release run SUCCESS; and the **independent cross-check**: the published amd64.deb downloaded and sha256-compared against SHA256SUMS.txt — byte-exact.

**The escaped-regex trap, first bite:** the version sweep's replace-all missed an *escaped* regex pin (`/0\.2\.0 is available/`) in a test file. The app test gate went RED pre-push — caught by the gate, fixed, amended, green. **The gate caught its own author.** (It bit again identically in the next release cycle — `/0\.2\.1 is available/` — and the gate caught it again. Traps repeat; detectors persist.)

---

## Day 10, evening — #100, the wire-read that saved the updater

**What happened:** during release verification (not a report — a wire-read), the published `latest.json` showed exactly one platform entry: `darwin-x86_64`. The product ships **Apple Silicon only**. Forensics on the vendored updater plugin source: `TargetNotFound` fires only when the feed is newer than the install — so *that day*, every installed Mac's update check failed silently. Root cause: v2 signing changed artifact shapes; the collector's v1-era shape heuristic guessed.

**The fix:** builder learns the v2 shapes (AppImage / NSIS exe / the macOS wrapper keyed by declared arch); an undeclared macOS bundle fails `UNKEYED` instead of guessing; `.sig` sidecars joined the attested set; the publish leg runs its self-test as a detector *before* building. Tag moved, re-driven, battery green — manifest key `darwin-aarch64`, signature byte-identical to the published sidecar.

**Rule it proves:** the story-gate lesson (verification-and-gates.md §1) and the user-view battery's worth, both in one lane. A gate fixtured on a story about the system passes while the real system breaks.

---

## Days 10–11 — #101, the distro move + the billing forensics

**What happened:** the source repo flipped private; wire facts: the baked feed unauth-**404** — every shipped install lost auto-update; README links auth-walled. Decision: private source + public distro. The lane: endpoint → distro feed; cross-repo publish via a scoped secret; bundle copyright corrected (a metadata chase had missed `bundle.copyright` still claiming "MIT community" on proprietary code — the closed-product mandate chased into the manifests); README/USERGUIDE repointed; one-final-manual-update note for pre-move installs; `DISTRO_TOKEN` sealed (value never printed).

**Then CI went all-red — and the loop's reflexes earned their keep.** Three RED runs, every job dead instantly, `fmt` included. Instead of debugging code: annotations read. *"The job was not started because recent account payments have failed or your spending limit needs to be increased."* Measured with the quota script: the private period had burned 5,708 quota units against a 2,000/month tier. **The code was never broken** — the meter was.

**The public-window play:** owner flipped the repo public (standard-runner minutes on public repos are free) → the quota debt and the billing trip dissolved → CI re-run GREEN 8/8 on the same commit → tag cut → release driven → and the day-11 unauth battery went ALL GREEN: feed 200 v0.2.2, platform keys = shipped hardware, signatures byte-matching, four real installers sha256-exact, release = latest.

**Rule it proves:** infra-RED vs code-RED triage (RB-1), the quota math habit, and that visibility is a *tool* with a checklist, not a mood.

---

## Day 11 — #102, nothing internal ships public

**What happened:** the owner reviewed the public release page and ordered it cleaned. First: a false alarm — the auto "Source code (zip)" contains only the distro repo's README (verified by downloading it). But the audit it triggered found the real thing: the publish glob `release_assets/*` had shipped **18 provenance sidecars** — 2 CycloneDX SBOMs (the full internal dependency closures) + 16 SLSA attestation bundles (DSSE envelopes embedding the *private* repo URL, workflow path, runner image, private commit SHAs) — and release notes citing internal issues.

**The lane (executed in the order the owner demanded):** DELETE the release first (204/204) — *then* analyze. Hygiene gate built with self-test riding CI (its clean fixture caught a gap in the gate itself before any evidence was posted). The fix: provenance generates into a private-only directory riding CI artifacts; publish hard-fails on any provenance-shaped file in the upload set; notes generator made user-facing (zero internal refs). CI GREEN 8/8 → tag moved → re-drive SUCCESS → **15 clean assets, zero provenance**. Same lane: the distro repo productized (description + 11 topics, product README, ported user guide, sanitized changelog) — zero engineering internals.

**Rule it proves:** allowlist-not-glob (the default-deny publish set), private artifacts for provenance, and RB-5's meta-lesson: the false-alarm report was wrong about the zip but *right* about the release.

---

## Day 11, late — the dark logo + the draft flip (the polish pass)

**What happened:** the owner, viewing the public README in dark mode: "the logo SVG you have now in readme is dark and GitHub readme is dark so it's not visible." Fix: the white-ink mono twin (same owner-approved geometry) uploaded to the distro repo + the header switched to `<picture>` `prefers-color-scheme` — adaptive ink, verified by rasterizing both variants on `#0D1117` and white. Caught in the same pass: the release tag was stale relative to main → the source zip shipped the old README. Re-pointed the tag — and the feed went **404 dark for ~2 minutes**: deleting a release's tag flips the release to *draft*; recreating the ref does NOT re-publish. PATCHed back (`draft: false`), full unauth battery re-run, all green.

**Rule it proves:** RB-4 (the draft flip — now a named runbook), the source-zip currency rule, and product-surface.md §7: the first report about a public surface comes from the owner in dark mode, and it finds in one look what ten authed reviews missed.

---

## The pattern across all eleven days

1. Every catch that mattered (the killer, the platform keys, the 18 sidecars, the billing block, the draft flip) was found by a **detector or a wire-read** — never by hoping.
2. Every fix was **evidenced at HEAD** before it was built and **verified at the user's chair** after.
3. Every incident produced a **record row**, and every repeated incident produced a **rule** (the escaped-regex trap: bitten twice, caught twice by the same gate, now a cabinet entry).
4. The owner's role stayed exactly what a big-tech *product owner's* role is: directives, reviews, billing actions, and the final dark-mode look — while the loop carried the engineering truth.
5. Nothing was lost across seven environment recycles, because nothing lived outside the repo, the record, and the vault.

That's the whole skill, proven. Run your project this way and the war stories write themselves.
