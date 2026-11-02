# Changelog

All notable changes to IUM are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/) with the 0.x pre-1.0 rules in
`docs/05_Engineering_Process.md` (§ Versioning policy).

Every entry cites its issue numbers — evidence lives in the tracker, not in
prose. An entry without verifiable issue numbers does not belong here.

## [Unreleased]

### Fixed
- #23 — `s3curl.py --abort-multiparts` crashed on `NameError: re` (module never
  imported `re`; the `s3-test.sh` exit trap discarded the crash via `|| true`) —
  multipart exit-hygiene has silently never worked; it now actually runs and the
  real-world suite exercises it for real.

### Added — M5: API freeze, docs & real-data evidence
- #17 — real-data e2e drill: a REAL playable H.264/AAC video (~23 MiB, ffmpeg-generated
  at test time) through the engine on HTTP loopback, TUS (tusd 2.10.1) and S3 multipart
  (SeaweedFS 4.51); byte-exact (`sha256(received) == sha256(source)`) and ffprobe-verified
  on the received side; `--with-real` harness flag on both server scripts; CI wired.
- #18 — real slow-connection drill: a throttled socket (server-paced reads, real TCP
  backpressure, no test clock) drives spec §4.2 eviction on live wire evidence —
  `chunk_evicted` journal event with measured bps vs pool median; impaired attempt
  abandoned mid-body, retry completes; the real video survives intact.
- #19 — public API freeze: surface reviewed and shrunk (~26 items: `sigv4` and `ids`
  modules private, eviction internals `pub(crate)`); every public item documented;
  `#![deny(missing_docs)]` enforced; `cargo doc --no-deps` warnings-as-errors lane in
  gate.sh and CI (3 OS).
- #20 — this changelog + the versioning policy (docs/05).

## [0.1.0] — 2026-11-02

The first frozen public surface (per #19): `JobManager`, `Supervisor`, `JobSpec`,
`JobRecord`, `JobSnapshot`, `SessionJournal`, the `TransferAdapter` contract,
the three transport adapters, `ChunkPool`/`Lease`/`ChunkState`, `EvictionPolicy`,
`RetryPolicy`, `JobState`, `EngineError`.

### Added — engine (M1, Sprint 0)
- Journal-backed upload engine: SQLite WAL session store with versioned migrations,
  transactional job+chunk insert, loud write failures (defects #1–#7 of the Sprint-0
  audit, fixed and pinned).
- Supervisor state machine `Created → Probing → Running ⇄ Paused → Finishing →
  Done | Failed (+ Unresumable, Canceled)`; every transition journaled before it
  executes (ADR-002).
- Dynamic chunk pool: planning with chunk_min/chunk_max clamps, leases, re-rentable
  partial confirms, tail race, resplit (spec §4.1/§4.2 patterns).
- Counted retries with per-error-class policy, backoff, `Retry-After` honoring,
  deterministic exhaustion → Failed (spec §5).
- Slow-chunk eviction: per-worker throughput watch, relative floor, strike counting
  (spec §4.2).
- HTTP one-shot transport over reqwest+rustls, streaming from disk, HEAD verify gate.

### Added — transports & drills (M3)
- #6 — hand-rolled SigV4 signer (`hmac`/`sha2`/`hex` only, no AWS SDK), pinned against
  official AWS test vectors, offline and pure.
- #7 — S3 multipart adapter: probe → CreateMultipartUpload, part-atomic resume,
  verify-as-commit-gate (ADR-010: ListParts reconcile + HEAD before Complete), session
  restore from the journal.
- #8 — planner consumes the adapter-declared `max_chunk_bytes` as a hard bound (ADR-011).
- #9 — S3 CI harness on 3 OS against pinned SeaweedFS 4.51 + kill-9 subprocess drill
  (112 MiB, SIGKILL mid-body, zero re-upload after resume) + nightly >100 MiB e2e.
- #10 — TUS 1.0.0 adapter against pinned tusd 2.10.1: byte-level resume, 409 realign
  absorbing drifted-but-server-acked bytes, read-only HEAD verify; 3-OS CI.
- #12 — `Action::Reattach`: `reattach` is a REQUIRED trait method (trybuild contract);
  manager replays the journaled probe token mid-transfer and at Finishing entry.
- #11 — TUS subprocess kill-9 nightly drill with a provably real kill window
  (journal 0 confirmed while the server holds half the file); byte-level zero re-upload
  pinned.

### Added — app shell (M4)
- #13 — adapter-declared `max_parallel`: TUS declares 1, the planner obeys (ADR-012) —
  concurrent TUS jobs can no longer 409-realign into `Unresumable`.
- #14 — the Tauri 2 + React shell unfrozen (ADR-006 amendment): workspace member,
  3-OS CI job (npm lane + shell clippy/tests), gate.sh app lane.
- #15 — runtime wiring: engine-side `Supervisor` (spawn/pause/resume/cancel), honest
  `Canceled` terminal state, `job_updated` journal-snapshot events; silent-corruption
  state parse fallback removed (unknown state strings fail loudly).
- #16 — Sprint-1 UI slice: create form (official dialog plugin), jobs table with live
  progress + honest badges + engine-gated actions, per-chunk inspector; pure `logic.ts`
  under vitest, wired into gate + CI.

### Changed
- CI: 3-OS matrix (ubuntu/macos/windows) for engine and shell; nightly fuzz
  (env-gated, mutation-proven) + coverage artifact + kill-9 drills (#4).

### Process
- Issue-driven loop with failing-test-first (docs/05); `scripts/gate.sh` gates the
  COMMITTED tree in a detached worktree after defect #11 (a push that never compiled);
  STATUS.md as the single source of truth (ADR-008).
