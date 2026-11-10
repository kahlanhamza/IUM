# Verification & Gates — the doctrine

*Reference for `zero-to-shipped`. Read when building or changing any gate, test, or verification claim.*

## 1. Gates are detectors, not stories

A gate is executable code that fails loudly when a standard is violated. It is not:
- a wiki page describing the standard,
- a checklist a human remembers,
- a test that exercises a *story* about the system instead of its real shape.

The distinction burned in twice:

**The story-gate.** The original updater-artifact collector's self-test was fixtured on v1-era wrapper bundles. The product moved to v2 signing (sign the installers themselves; wrap only macOS). The gate kept passing while the real artifact names had changed shape underneath it — it tested a story about the system, not the system. The fix made the gate learn the v2 shapes and, crucially, **fail loud on an unknown shape** (`UNKEYED`) instead of guessing. A gate that can be silently bypassed by reality changing is decoration.

**The fixture gap.** A brand-new publish-hygiene gate was self-tested pre-commit; its clean fixture was missing the very file class it existed to catch (an app-SBOM line). Caught *before* posting RED evidence — because the gate had a self-test that insisted on seeing a RED in its mirror. **Every gate needs a self-test that proves it fires on a bad input and passes on a good one, and the self-test rides CI.**

## 2. Gate design rules

1. **Fail loud, fail named.** `UNKEYED`, `PROVENANCE_IN_RELEASE_ASSETS` — the failure names the violation class, not just exit code 1.
2. **Closed by default.** Allowlists over denylists (see release engineering). Unknown shapes are failures, not guessing games.
3. **Deterministic.** No wall-clock, no network in the gate's decision path where avoidable; if network is inherent (a feed check), the gate prints exactly what it saw.
4. **Self-test both directions:** a bad fixture must RED, a clean fixture must GREEN. One-directional self-tests rot toward always-green.
5. **Ride CI.** A gate that isn't exercised in CI decays silently — the first run after three weeks of drift is the day you need it most.
6. **Run on the committed tree.** Gates on a dirty worktree certify the wrong bytes. Commit first, gate second, push third.
7. **Cheap enough to always run.** The full local battery in the source project (format + 166 app cases + the engine suite + doc lint + specialized gates) ran in minutes and therefore *always* ran.

## 3. The verification ladder (for any claim)

Claims climb a ladder; never jump rungs:

1. **Compiles / formats** — necessary, worth nothing.
2. **Unit suite green** — the logic is right in the small.
3. **Integration/e2e green** — the pieces agree (100-jobs load e2e; the axe a11y suite; the installer-size budget gate).
4. **The wire agrees** — the actual artifact/endpoint says what we claim (bytes, status codes, platform keys).
5. **The user's chair agrees** — unauthenticated, plain tooling, exactly what a stranger gets.

Most projects stop at 2. The release bar demands 4 and 5 for anything user-facing. The unauth battery (release engineering, phase D) is rung 5 codified.

**Byte-identity pattern** (the workhorse of rungs 4–5): when verifying a signature/checksum surfaced in one place against its published artifact, compare *bytes*, not "they should match" — decode both, diff both, print lengths on mismatch. In the source project the manifest's darwin signature was proven byte-identical to the published `.sig` asset (both base64 layers decoded) — that's rung-4 evidence you can bet the updater on.

## 4. RED/GREEN evidence discipline

- **RED before fix:** the receipt that the gap existed at HEAD (gate output, reproduced bug, user-view 404). Posted to the issue.
- **GREEN after fix:** the receipt that the gap is closed (run IDs, checks, byte-comparisons). Posted to the issue.
- **Instrument check:** before trusting either, confirm the instrument works (the self-test rule). Evidence from a broken instrument manufactures false confidence in *both* directions.
- **The gate that caught its own author:** twice, an automated replace-all missed an *escaped* regex pin (e.g. `/0\.2\.0 is available/` in a test file — the literal backslash survives the replacement). The test gate went RED **pre-push**, both times. That's the system working: the human/agent edits, the gate disbelieves. Design your gates assuming your own edits are untrustworthy — they are, statistically.

## 5. What verification is NOT

- **Not "CI passed."** CI passing means the checks *that exist* passed on the tree *that was pushed*. The #100 lesson: CI was green for a release whose updater feed matched no shipped hardware. CI is a floor, not a verdict.
- **Not "the build succeeded."** A build can succeed and publish the wrong assets. The hygiene gate exists because build success and *ship correctness* are different properties.
- **Not "I tested it locally."** Local testing under your privileges proves nothing about the unauth surface. The user-view battery exists precisely because *you* are the wrong person to test your own release: you have credentials, a warm cache, and good intentions. The user has none of those.

## 6. A worked example (the platform-key lane, #100)

The lane that best shows the ladder in motion:

1. **Wire-read during release verification** (not a report): the published `latest.json` carried exactly one platform entry, `darwin-x86_64` — while the product ships Apple Silicon only.
2. **Forensics on the plugin source** (vendored, read, not guessed): the updater errors `TargetNotFound` only when the feed is *newer* than the install — meaning a v0.2.0 user's update check was failing *that day*, and Windows/Linux would join at the next bump.
3. **Root cause in the builder:** v2's artifact shapes changed (installers signed directly; macOS wrapped in an arch-less `.app.tar.gz`); the collector's shape heuristic guessed wrong.
4. **RED at HEAD:** the collector mis-keys on the real artifact names.
5. **Fix:** builder learns v2 shapes; undeclared macOS bundle → `UNKEYED` failure instead of a guess; `.sig` sidecars join the attested set; the publish leg runs the self-test as a detector *before* building.
6. **GREEN:** simulated key map on real artifact names → CI 13/13 → tag moved → re-drive → battery green (manifest key `darwin-aarch64 → IUM.app.tar.gz`, signature byte-identical to the published sidecar).
7. **Record:** the row, including the v1-story-gate lesson.

Total: one issue, one commit, one re-drive, zero broken users. Compare with the counterfactual: shipping it, and discovering via bug reports that every installed Mac stopped checking for updates.
