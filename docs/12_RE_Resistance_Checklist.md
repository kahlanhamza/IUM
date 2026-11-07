# 12 · The RE-Resistance Checklist — the closed-release bar

The owner amendment (issue #66, comment 6673366729) restored reverse-engineering
resistance as a real bar: **at ship time this repo goes CLOSED**, and a closed
release must clear an honest bar — without aggressive obfuscation, which breaks
crash triage and buys nothing against native binaries. This document is the
final RE-review checklist: read it, run its gates, tick it — THEN flip private.

Status: all four pins are enforced in-tree today. The `--bin` gate's real
verdict lands in release.yml at the next tag (same honesty contract as the
app-e2e lane: capability-gated local skip, enforced at release).

---

## Pin 1 — release builds stripped of debug symbols

**The pin.** `[profile.release] strip = "symbols"` in the root `Cargo.toml`
(workspace-wide: the shell binary `IUM`, the drill child, the e2e bins). The
shipped binary carries no symbol table and no debug info — no function names
recoverable by `nm`/IDE, no DWARF.

**Verified by.** `scripts/re_resistance_check.sh`:
- `--repo` (ci.yml, every push): the profile pin exists in the committed tree.
- `--bin target/release/IUM` (release.yml, per-OS, before upload): `file(1)`
  reports `stripped` (unix); the RSDS codeview marker is absent (windows
  fallback). A binary that is not stripped FAILS the release before anything
  is uploaded.

**Current status.** The pin landed with this checklist (lane 8, #76). Cargo's
default was `strip = "none"` — the symbol table shipped until now. Verified
GREEN on a real release-profile build (the drill-child binary: `file` →
`stripped`).

## Pin 2 — no panic paths print internal paths

**The pin.** No panic/expect/unwrap message interpolates a filesystem path —
no user-machine paths, no absolute build paths, in printed panics or in
telemetry events.

**Verified by.** `scripts/re_resistance_check.sh --repo`: a source audit over
`ium-core/src` + `ium-app/src-tauri/src` fails on any
`panic!/expect!/unwrap!` message containing `.display()` or `{:?}` path
interpolation. Plus the build-machine scan below.

**The deliberate distinction (read before flipping).** The telemetry panic
event (#75 charter, "The honest limits") carries the panic's
**compile-time workspace-relative source location** (e.g.
`ium-app/src-tauri/src/lib.rs:390`) plus the verbatim panic message. That is
not a user-machine path and not a build-machine path — it is how every native
product reports crashes for triage, and the panic location stays in the LOCAL
buffer unless consent is on AND a transport exists (none ships today; the
sink is a pinned owner ask). The std default hook prints the same
location-class to stderr. This is the intended, documented behavior — do not
"fix" it out of the binary during the flip review.

**Current status.** The audit is clean (executed at lane-8 build time;
zero matches).

## Pin 3 — secrets never embedded in the binary

**The pin.** No credential VALUE is compiled into any shipped binary.
Credentials enter only through runtime env or the vault (`*_from_env`
constructors: `FEISHU_*`, `RCLONE_CONFIG_*`, `IUM_S3_*`, TeraBox cookies,
mega) — the `.secrets/` contract plus never-commit-never-print keep the repo
private-flippable at any moment (clean history, keys only in GH Actions
secrets / `.secrets/`).

**Verified by.** `scripts/re_resistance_check.sh --bin <path>`: a
value-shaped scan over the built binary — GitHub token prefixes
(`github_pat_`, `ghp_/gho_/ghs_/ghu_`), AWS (`AKIA…`), `sk-…` keys, Slack
(`xox…`), PEM private-key headers, TeraBox cookie VALUES (`ndus=…`,
`browserid=…`, `csrfToken=…`, `jsToken=…` with a value-shaped tail). The
scanner NEVER prints matched values (a scanner that echoes secrets is itself
a leak) — pattern names and hit counts only.

**Why names are not scanned.** Cookie/param NAMES (`ndus`, `csrfToken`,
`jsToken`, `browserid`) are protocol vocabulary the adapters legitimately
contain — they must be IN the binary to speak the protocol. What must never
be there is a value. The selftest proves exactly this distinction: planted
values are detected; name-only fixtures pass.

**Current status.** The env-contract holds (wire-fact verified at lane-8
build time); the release gate re-verifies per build.

## Pin 4 — no build-machine paths in the binary

**The pin.** No build-host fragment (`/home/`, `/root/`, `/Users/`,
`C:\Users\`) survives into the shipped binary — a privacy bar on the build
environment and a guarantee that no absolute paths leak through
debug/location strings.

**Verified by.** The same `--bin` scan (fixed-string check per fragment).

**The real catch this pin earned (why it exists).** The FIRST real
release-profile build under this lane leaked: registry-dependency panic
locations embed the build host's ABSOLUTE registry path
(`/home/z/my-project/.toolchain/cargo/registry/src/...`) as `Location`
strings — the `--bin` scan flagged `/home/` on the drill-child binary. The
stable fix is `--remap-path-prefix` (cargo's `trim-paths` profile option is
nightly-only on the stable toolchain and would break every CI build):
`scripts/rustflags_remap.sh` computes this machine's CARGO_HOME registry
prefix the way cargo resolves it (env override, else the platform default —
NOT the rustup sysroot, which lives under a different tree) and emits the
separator variants; the release.yml build step exports it into RUSTFLAGS;
the `--bin` gate verifies the OUTPUT. After the remap: the same scan is
GREEN — crate-relative registry tails remain for honest triage, the build
host does not.

**Current status.** Enforced at release; verified GREEN on a real
release-profile binary after the remap (lane-8 build-time evidence).

---

## The private-flip procedure (run at ship, in order)

1. **Freeze and tag.** Final tag; release.yml builds; `--bin` gate GREEN per
   OS (stripped + no secret values + no build paths); installers pass the
   size budget; SBOM + provenance verified (`release_supply_chain_check.sh`).
2. **Burn check.** Pickaxe the full history once more for token patterns
   (the pre-public-flip audit is the precedent — zero real tokens expected;
   the `.githooks/pre-commit` block regex stays).
3. **Secrets inventory.** Everything credential-shaped lives in GH Actions
   secrets or `.secrets/` (local-only). Anything that ever landed in chat or
   history is rotated/burned (the standing rotation rule).
4. **Read this checklist end-to-end** — all four pins' "Current status" are
   GREEN at the ship tag, no open exceptions.
5. **Flip.** Settings → Danger Zone → Change visibility → Private.
6. **Post-flip note.** CI drops to the private free tier (~2,000 min/month):
   the workflow set stays as-is; if minutes pressure appears, cut matrices
   last and gates never (the CI-economics decision, amendment item 3).

## What this bar does NOT promise

No obfuscation, no name mangling, no control-flow flattening, no anti-debug
tricks. Native binaries stripped of symbols, carrying no secrets and no
internal paths, clear an honest bar. The updater channel (minisign-signed
artifacts) and the SBOM/provenance attestations (#71) remain the
supply-chain half of the trust story; per-OS code signing stays the pinned
owner ask (certs cost money).
