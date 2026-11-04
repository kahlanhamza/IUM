<div align="center">

# IUM — Internet Upload Manager

**The missing desktop twin of IDM / aria2 / Motrix / FDM — for uploads.**

Resumable, segmented, multi-protocol uploads that survive slow networks, sleep,
crashes and reboots — with *honest* badges where physics says a protocol cannot resume.

[![ci](https://github.com/kahlanhamza/IUM/actions/workflows/ci.yml/badge.svg)](https://github.com/kahlanhamza/IUM/actions/workflows/ci.yml)
[![nightly](https://github.com/kahlanhamza/IUM/actions/workflows/nightly.yml/badge.svg)](https://github.com/kahlanhamza/IUM/actions/workflows/nightly.yml)
[![stress](https://github.com/kahlanhamza/IUM/actions/workflows/stress.yml/badge.svg)](https://github.com/kahlanhamza/IUM/actions/workflows/stress.yml)
![rust](https://img.shields.io/badge/rust-stable%201.99-blue)
![tests](https://img.shields.io/badge/tests-152%20core%20%2B%209%20UI-brightgreen)
![platform](https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20Windows-lightgrey)

</div>

---

Every download manager solves *"a 5 GB file on a flaky 2 Mbps line"* — for **downloads**.
Uploads still get one fragile `POST` that dies at 90% and starts over. IUM applies twenty
years of download-manager engineering — chunk pools, adaptive split, slow-link eviction,
endgame racing, crash-safe journals — to the upload direction, and pairs it with a policy
the industry usually leaves implicit: **never claim what was not verified**.

## Highlights

| | |
|---|---|
| **Crash-safe by construction** | SQLite WAL journal is the only truth; state is read from it, never from caller arguments (ADR-002). Transitions are journaled *before* they execute — kill -9 mid-transfer resumes byte-exact with **zero re-upload** (pinned by two nightly kill-9 drills). |
| **Verify-before-declare** | `verify` is a *required* trait method — compile-fail-pinned by a trybuild contract test. A job reaches Done only after the server confirms the assembled object. |
| **Three real transports** | HTTP one-shot (streaming PUT), S3 multipart (hand-rolled SigV4, zero SDK), TUS 1.0.0 (byte-level resume with 409 realign). Each declares its capabilities; the UI renders honest badges from them. |
| **Adaptive engine** | Dynamic chunk pool, slow-connection eviction voted on measured throughput, shrink-only adaptive resplit, per-error-class retry budgets — every path ends in Done or Failed, never in silence. |
| **Proven against reality** | Real-network impairment models (packet loss, RTT/jitter, mid-body RST), B/s-class uplink ladders, overnight long-haul legs with fresh-journal replay, resource-growth soak bounds, real video files through every adapter. |
| **Real shell, really driven** | Tauri 2 + React desktop app exercised end-to-end on CI by a hand-rolled W3C WebDriver suite (WebKitGTK under xvfb): create → live progress → pause/resume → Done → per-chunk inspector → cancel commits nothing. |

## Architecture

```
┌────────────────────────── ium-app (Tauri 2 + React) ──────────────────────────┐
│  CreateJobForm · JobsTable (live %, speed, honest badges) · InspectorDrawer   │
│  logic.ts — pure functions over journal snapshots (vitest-pinned)             │
└───────────────────────────────┬───────────────────────────────────────────────┘
                                │ typed IPC commands + job_updated events
┌───────────────────────────────▼───────────────────────────────────────────────┐
│  Supervisor  spawn / pause / resume / cancel — journal-truth state machine    │
├───────────────────────────────────────────────────────────────────────────────┤
│  JobManager  chunk pool · leases · eviction · resplit · counted retries       │
├───────────────────────────────┬───────────────────────────────────────────────┤
│  TransferAdapter trait        │        SessionJournal (SQLite WAL)            │
│  ├─ HttpOneShot  (PUT)        │  jobs · chunks · events — versioned           │
│  ├─ S3Multipart  (SigV4)      │  migrations · atomic transitions              │
│  └─ Tus 1.0.0    (PATCH)      │  ENOSPC-hardened: state+event in ONE tx       │
└───────────────────────────────┴───────────────────────────────────────────────┘
```

**Design invariants** (enforced by tests, recorded in `docs/06_Decisions_ADR.md`):

1. The journal is the only truth — adapters may keep sessions, the journal owns state.
2. Transitions are journaled **before** they execute; a crash between the two is recoverable.
3. `verify` cannot be silently inherited — the contract test breaks the build if it is.
4. Retries are counted; every failure path terminates in **Done** or **Failed**.
5. Cancel leaves **no server object** — pinned on the wire, not in prose.

## Quick start

```bash
# Engine — builds anywhere with a Rust stable toolchain
cargo test -p ium-core          # 152 tests: unit + wire + drills + contracts
cargo clippy -p ium-core --all-targets -- -D warnings

# Desktop shell
cd ium-app && npm ci && npm run build
cargo run -p ium-app --bin ium-app   # or: npm run tauri dev

# Full gate on the committed tree (what CI runs)
bash scripts/gate.sh

# Live protocol rehearsal against pinned real servers (~70 MB downloaded once)
bash scripts/s3-test.sh          # SeaweedFS 4.51  → 23/23 (+ --with-real video drill)
bash scripts/tus-test.sh         # tusd 2.10.1     → 12/12 (+ --with-drill kill-9)

# Real-shell e2e (Linux: xvfb + webkit2gtk-driver + tauri-driver)
bash scripts/app-e2e.sh
```

## Transports & honesty

| Adapter | Protocol | Resume | Mid-flight confirm | Notes |
|---|---|---|---|---|
| `HttpOneShot` | plain `PUT` | ⚠️ restart-from-zero | completion only | streaming from disk, `Retry-After`, ETag capture, HEAD verify |
| `S3Multipart` | S3 multipart | ✅ per-part | per-part | zero-dependency SigV4, part-map reconcile before `Complete` (ADR-010) |
| `Tus` | TUS 1.0.0 | ✅ byte-level | per-`PATCH` server ack | 409 offset realign absorbs drifted bytes; session loss → counted reattach |

Where a server physically cannot resume, the UI says **One-shot ⚠️** instead of pretending.
The badge is derived from the adapter's declared capabilities — never hard-coded.

## How it is tested

| Lane | What runs | Where |
|---|---|---|
| Offline core | **152 tests** — unit, chunk-algebra fuzz, integration drills, supervisor, journal contracts (incl. compile-fail) | every push, 3 OS |
| Live wire | 23/23 S3 vs **SeaweedFS 4.51**, 23/23 vs **rclone 1.75.2**; 12/12 TUS vs **tusd 2.10.1**, 12/12 vs **@tus/server 2.4.7** | every push (Linux), 5 real server implementations |
| Shell e2e | real WebDriver session through tauri-driver → WebKitGTK; both upload legs asserted against **server truth** (recorder record + HEAD), not UI promises | every push (ubuntu) |
| Nightly | invariant fuzz at 400×, coverage artifact, **kill -9 drills** (S3 + TUS: resume with zero re-upload), >100 MiB end-to-end | scheduled |
| Stress bench | real 308 MiB film S1–S5; impairment models `loss` / `rtt` / `reset`; B/s uniform ladders; long-haul with journal replay + RSS/journal bounds | dispatchable (`stress.yml`) |

Numbers the drills pin: kill -9 at 25% → resume moves exactly the missing bytes;
16,391 journal events → ≈153 B/event linear, WAL checkpoints to 0, RSS +5.5 MiB;
eviction fires on *measured* throughput (a 32 KiB/s victim dies in 20 s instead of a
234 s wait-out) and never on merely-latent links.

## Repository map

```
ium-core/            the engine — no UI, no cloud SDK deps
  src/chunk.rs         chunk algebra: plan / lease / evict / resplit / tail-race + fuzz
  src/journal.rs       SQLite WAL store: jobs / chunks / events, versioned migrations
  src/manager.rs       JobManager: journal-validated transitions, crash recovery
  src/supervisor.rs    spawn / pause / resume / cancel over Arc<JobManager>
  src/eviction.rs      throughput-voted slow-link eviction (never churns latent links)
  src/adapter*.rs      HTTP one-shot · S3 multipart · TUS (+ capability registry)
  src/sigv4.rs         hand-rolled AWS SigV4 (external-vector pinned)
  src/bin/             loopback_recorder · drill children (kill-9 subjects)
  tests/               integration, wire suites, drills, contracts, soak bounds
ium-app/             Tauri 2 + React shell (src-tauri capabilities, vitest logic tests)
ium-e2e/             real-shell WebDriver suite (hand-rolled W3C client, env-gated)
scripts/             harnesses: s3-test.sh · tus-test.sh · app-e2e.sh · stress &
                     long-haul drivers · throttle_proxy.py (impairment models)
docs/                numbered: architecture teardown · MVP spec · ADRs · roadmap ·
                     engineering process · boundary register
rea-lab/             clean-room evidence dossiers (binaries stay out of git)
```

## Documentation

| Doc | Contents |
|---|---|
| [docs/01_Architecture_Teardown_v2.md](docs/01_Architecture_Teardown_v2.md) | REA study: aria2, XDM, Motrix, FDM 6.35, IDM 6.43 |
| [docs/02_IUM_MVP_Spec_v1.md](docs/02_IUM_MVP_Spec_v1.md) | behavior contract, stack decision, data model |
| [docs/04_Roadmap.md](docs/04_Roadmap.md) | milestones M0–M6, each closed with evidence |
| [docs/05_Engineering_Process.md](docs/05_Engineering_Process.md) | the loop: issue → design note → RED → fix → gates → push |
| [docs/06_Decisions_ADR.md](docs/06_Decisions_ADR.md) | accepted architecture decisions |
| [docs/08_Boundary_Register.md](docs/08_Boundary_Register.md) | entity map: every seam and what breaks there first |
| [CHANGELOG.md](CHANGELOG.md) | Keep-a-Changelog, every entry cites its issue numbers |

## Method: clean-room REA

We study how commercial and open download managers are *built* — file layout, module
names, protocol vocabulary, import tables — and write an **independent specification**.
No proprietary code is copied, decompiled, or redistributed; acquired binaries never
enter git. Every architecture claim in `docs/` cites a dossier in `rea-lab/out/`.

## Contributing

This repository follows a strict evidence-first loop (full text in
`docs/05_Engineering_Process.md`):

1. **Issue first** — no code without an issue; design notes precede non-trivial changes.
2. **Failing test first** — bugs are reproduced as RED tests, pasted on the issue.
3. **Minimal fix** — no drive-by refactors; unrelated findings get their own issue.
4. **Full gates every time** — fmt · clippy `-D warnings` · tests · docs, on the
   *committed* tree (`scripts/gate.sh`), not the working tree.
5. **One logical commit per issue**, `Closes #N` in the body; `STATUS.md` is updated
   and pushed with every task — GitHub is the memory, not chat history.

## Security

Secrets (PAT, Kaggle token) live only in gitignored `.secrets/` files with `0600`
permissions — never committed, never printed. Harness servers bind loopback only.
The CI matrix runs on GitHub-hosted runners; pinned server downloads are
checksum-verified. Report vulnerabilities via a private GitHub security advisory.

## Status & roadmap

Milestones **M0–M6 are complete** — including the M6 readiness probe generation
(real-network impairment · server diversity · long-horizon & resource honesty ·
real-shell e2e), closed on CI run evidence. All CI lanes green.
**Current: M7 — Speed architecture** (owner mandate, measured field profiles): a link
prober that classifies the bottleneck at probe time and picks lane + fleet from
measurement; the tus idle-progress timeout (the 600 s total wall is non-convergent on
KB/s links); own-drift absorption in the realign dance; a lane-aware chunk floor; a
permanent throttled-link ladder in CI; and a measured research track (multipath
aggregation, FEC). After M7: Sprint-2 UX, more transports (WebDAV, Google Drive),
account/OAuth layer, and release packaging toward **v0.1.0**.

---

*Private during bootstrap — no license granted yet. Versioning follows semver with
the 0.x rules in `docs/05`; notable changes are recorded in [CHANGELOG.md](CHANGELOG.md).*
