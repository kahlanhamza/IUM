# IUM — Internet Upload Manager

> The missing desktop twin of IDM/aria2/Motrix/FDM — for **uploads**.
> Resumable, segmented, multi-protocol uploads that survive slow networks, sleep, crashes and reboots.

**Status:** Sprint 0 shipped — `ium-core` live (22 unit + 8 integration tests, clippy-clean, 3-OS CI) · Tauri shell scaffolded · **Repo:** private during bootstrap

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
  state.rs               supervisor state machine (pattern #5)
  journal.rs             SQLite WAL session store: jobs/chunks/events (pattern #6)
  adapter.rs             TransferAdapter trait + Capability flags + registry (pattern #7)
  manager.rs             JobManager facade: journaled transitions, verify-before-declare (#10)
  retry.rs               per-error-class policy table (pattern #9)
  ids.rs                 ULID-shaped job ids
  tests/integration.rs   mock-adapter drills: run/pause/resume/replay/one-shot/retry
ium-app/                 Tauri 2 + React shell (source scaffold; desktop builds it)
  src/api.ts             typed IPC contract (command mirrors)
  src-tauri/             commands: engine_status/create_job/list_jobs/snapshot/pause/resume
.github/workflows/ci.yml  fmt + clippy + tests on ubuntu/macos/windows
docs/
  01_Architecture_Teardown_v2.md   ← REA study: aria2, XDM, Motrix, FDM 6.35, IDM 6.43
  02_IUM_MVP_Spec_v1.md            ← stack decision, architecture, data model, roadmap
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

## Build (desktop)

```bash
# engine only (works anywhere with rustup stable)
cargo test -p ium-core

# desktop shell (needs WebKitGTK on Linux / Xcode / MSVC)
cd ium-app && pnpm install
pnpm tauri icon path/to/icon.png   # once, before first bundle
pnpm tauri dev
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
