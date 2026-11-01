# IUM — Internet Upload Manager

> The missing desktop twin of IDM/aria2/Motrix/FDM — for **uploads**.
> Resumable, segmented, multi-protocol uploads that survive slow networks, sleep, crashes and reboots.

**Status:** Sprint 0 (engine-first) shipped — `ium-core` hardened: **45 tests green** (28 unit + 13 integration + 4 loopback-HTTP end-to-end), fmt + clippy clean at `-D warnings`, 3-OS CI · Real `HttpOneShotAdapter` streams bytes over the wire · Tauri shell parked until the engine ground is complete · **Repo:** private during bootstrap

## Why

Every download manager solves "a 5 GB file on a flaky 2 Mbps line" for **downloads**.
Uploads still get one fragile `POST` that dies at 90% and starts over.
IUM applies 20 years of download-manager engineering (segment pools, adaptive split,
endgame racing, crash-safe journals) to uploads — with honest badges where a protocol
physically cannot resume (`Resumable ✅ / One-shot ⚠️`).

## Repo map

```
Cargo.toml               cargo workspace (ium-core; ium-app joins on desktop)
ium-core/src/
  chunk.rs               chunk algebra: plan/lease/evict/resplit/tail-race (patterns #1,2,3,9)
                         + invariant fuzz test (10k+ random ops, coverage never breaks)
  state.rs               supervisor state machine (pattern #5)
  journal.rs             SQLite WAL session store: jobs/chunks/events (pattern #6)
                         + versioned migrations (PRAGMA user_version) + atomic inserts
  adapter.rs             TransferAdapter trait + Capability flags + registry (pattern #7)
  adapter_http.rs        HttpOneShotAdapter: streaming PUT from disk (rustls),
                         Retry-After, ETag capture, HEAD verify-before-declare (#10)
  manager.rs             JobManager facade: journal-validated transitions, single-driver
                         guard, crash recovery from any journaled state, partial-confirm resume
  retry.rs               per-error-class policy table (pattern #9)
  ids.rs                 ULID-shaped job ids
  tests/integration.rs   disk-backed drills: resume, retry-exhaustion, pause, crash recovery,
                         stale-lease reclaim, double-run guard
  tests/http_one_shot.rs REAL loopback TCP server (chunked-encoding decode): happy path,
                         500-exhaustion, 429+Retry-After, lying-server verify gate
ium-app/                 Tauri 2 + React shell (source scaffold; parked — engine comes first)
  src/api.ts             typed IPC contract (command mirrors)
  src-tauri/             commands: engine_status/create_job/list_jobs/snapshot/pause/resume
.github/workflows/ci.yml  fmt + clippy + tests on ubuntu/macos/windows
docs/
  01_Architecture_Teardown_v2.md   ← REA study: aria2, XDM, Motrix, FDM 6.35, IDM 6.43
  02_IUM_MVP_Spec_v1.md            ← stack decision, architecture, data model, roadmap
  03_Sprint0_Engine_Record.md      ← audit findings: 7 real bugs found & fixed, with tests
rea-lab/
  out/                             ← evidence dossiers + raw scans (committed)
  targets/                         ← acquired binaries (gitignored — not redistributable)
scripts/
  idm_pe_scan.py                   ← clean-room PE scanner
  elf_strings_scan.py              ← clean-room ELF strings scanner
  magic_carver.py                  ← installer container carving
  preserve.sh                      ← commit+push snapshot of all work
  gh-push.sh                       ← token-handling push helper (token never in repo)
```

## Build (engine first — this is the product right now)

```bash
# engine only (works anywhere with rustup stable)
cargo test -p ium-core
cargo clippy -p ium-core --all-targets -- -D warnings

# desktop shell (parked until the engine ground is solid)
cd ium-app && npm install
npm run tauri dev
```

## Method (clean-room REA)

We study how commercial/open download managers are *built* — file layout, module names,
protocol vocabulary, import tables — and write an **independent specification**.
No proprietary code is copied, decompiled, or redistributed. Acquired binaries stay out of git.

## Stack (decided — see spec for rationale)

**Rust core (tokio) · Tauri 2 shell · TypeScript + React UI · SQLite (WAL) session journal**

## Working rules

1. `scripts/preserve.sh` after every work session — nothing lives only in a sandbox.
2. Evidence before opinion: every architecture claim in `docs/` cites a dossier in `rea-lab/out/`.
3. Honesty badges in UX where physics says no (pure-POST endpoints).
