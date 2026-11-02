# Changelog

All notable changes to IUM are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/) with the 0.x pre-1.0 rules in
`docs/05_Engineering_Process.md` (§ Versioning policy).

Every entry cites its issue numbers — evidence lives in the tracker, not in
prose. An entry without verifiable issue numbers does not belong here.

## [Unreleased]

### Fixed
- #26 — Finishing is no longer past canceling. The first free-cloud stress run
  (GitHub Actions, real 308 MiB film) failed S4-cancel with `Finishing -> Canceled
  is not a legal transition`: four parallel parts confirm in ONE step on a fast
  link, so the cancel driver's first sight of progress already coincided with the
  verify gate — and on a real KB/s uplink that gate is seconds of genuine network
  I/O (reattach → ListParts reconcile → CompleteMultipartUpload) during which
  Cancel was refused. Fix: `Finishing → Canceled` is legal; the gate honors a
  journal-Canceled state at ENTRY, before any verify/Complete I/O, so a canceled
  upload never completes the object; the razor window (cancel raced the verify
  call, the server completed anyway) gains explicit tolerance — the journal keeps
  the user's `Canceled`, a `complete_after_cancel` event records the server truth,
  and no false Done is ever declared. Verify-before-declare untouched. Pinned by
  `tests/finishing_cancel.rs` (RED at the pre-fix HEAD with the exact CI error).
- #22 (harness, first full execution) — the committed drill had never actually run
  end-to-end; its first real run caught three harness defects, engine clean every
  time. (1) `throttle_proxy.py` deadlocked EVERY proxied PUT: it parsed the request
  head and silently discarded `rest` — the first body slice that arrives glued to
  the head in one TCP read — then waited for body bytes the client had already sent
  while the client waited for a response that could never complete (240 s probe:
  frontier 0, no `VICTIM:` line ever). Fix: `rest` is forwarded first and counted
  in the pacing budget; client-mid-body abandonment (the eviction path) tears both
  ends down instantly; response reads are bounded. (2) A 10 MiB failure asset plans
  ONE chunk (`min(4 workers, floor(size / 5 MiB CHUNK_MIN))`), so the victim part
  `partNumber=2` cannot exist and the hostile-network scenario silently runs
  unimpeded — the driver now fails loud below a 20 MiB failure-asset floor.
  (3) `s4pause` raced the cooperative-pause contract ("takes effect between
  leases"): it asserted journaled `Paused` immediately and later let a terminal job
  reach `pause()`. Fix: one journal snapshot (state AND frontier) is the decision
  input; terminal-before-pause is the honest loud "upload too fast" outcome; the
  Paused wait is bounded and records the observed pause latency as evidence.
  Also: the evidence elapsed-extractor now reads stdin (string interpolation
  mangled the eviction payloads' escaped quotes), and the `stress.yml` free-cloud
  bench gained dispatch inputs (`victim_rate`, `victim_seconds`) so any real KB/s
  uplink can be modeled per run. The second cloud run also showed plain racing
  kills cannot land on fast links (parallel parts confirm in ONE step; all 3
  attempts finished before the kill, the post-completion kill hitting an
  already-Done journal) — S2 now uses the child's `--allow-calls 1` wedge for a
  DETERMINISTIC mid-flight shape (one part confirmed, the rest frozen at entry,
  kill window unbounded on any link), and `s3_drill_child` gained `--workers`.
  Full local S1–S5 GREEN vs pinned SeaweedFS 4.51
  at the owner's 32 KiB/s anchor; evidence on the issue.
- #25 — eviction no longer goes blind when the impaired connection is the only one
  left active. The #22 real-world S3 drill (32 KiB/s victim link) caught it live:
  every healthy part confirmed sub-window (never sampling a window), finished leases
  dropped their `live_bps` entry, and `below_floor`'s fewer-than-two-samples guard
  then refused to judge the lone laggard — the victim part crawled the FULL wait-out
  (~472 s for a 15.5 MB part at 32 KiB/s) with zero `chunk_evicted`. Fix: completed
  leases keep ONE vote each (freshest window rate, else the lease mean; zero-progress
  confirms never vote), the floor computes over live ∪ completed samples, and votes
  are capped at the fleet's best live window rate — the cap added after the
  integration eviction drill caught the un-capped first draft evicting HEALTHY
  streamers (an un-capped sub-window mean inflated the median past every honest
  rate, exhausting the retry budget). The `len < 2` guard now means "true lone
  fleet" (e.g. TUS `max_parallel = 1`). Pinned by `tests/lone_worker_eviction.rs`.
- #24 — verify-repair in the finish gate: when ADR-010 verify REFUSES a resumed
  upload because the SERVER's part list cannot assemble (SeaweedFS keeps a
  duplicate entry for a completed-but-unacknowledged PUT that the resume re-sent),
  the gate now makes ONE counted, journaled repair — re-probe (fresh multipart
  session, journaled per ADR-002), reset every chunk to Pending over the SAME
  spans, re-drive, re-verify — instead of ending an otherwise-perfect upload as
  Failed. Verify-before-declare is not weakened: Complete fires only after a clean
  reconcile on the fresh session; a second refusal is terminal (honest Failed);
  non-resumable badges never repair. The verify refusal detail now carries the
  server-listed parts (bounded to 32) so refusals carry wire truth. Pinned by
  `tests/verify_repair.rs` (repair-once → Done, bounded → Failed, clean path
  byte-identical).
- #23 — `s3curl.py --abort-multiparts` crashed on `NameError: re` (module never
  imported `re`; the `s3-test.sh` exit trap discarded the crash via `|| true`) —
  multipart exit-hygiene has silently never worked; it now actually runs and the
  real-world suite exercises it for real.

### Added — M5: API freeze, docs & real-data evidence
- #22 — real-world stress drill: a real 308 MiB 1080p film (Big Buck Bunny, Blender
  Foundation, CC-BY, archive.org — sha256-pinned) plus two smaller real films through
  five engine-driven scenarios vs pinned SeaweedFS 4.51 with the production
  S3MultipartAdapter: S1 plain upload → byte-exact read-back; S2 crash & resume
  (SIGKILL mid-flight at a real moment, fresh process reattaches); S3 hostile network
  (transparent TCP throttle proxy in front of the REAL server — KiB/s-scale victim
  link, `IUM_S3_VICTIM_RATE`/`_SECONDS` tunable to the owner's real uplink; production
  eviction defaults must fire `chunk_evicted` on live wire numbers); S4 user actions
  (pause freezes the wire provably, resume → Done; cancel → `Canceled` journaled and
  the server never completes the object — HEAD 404); S5 concurrent three-film fleet.
  Every scenario prints one machine-checkable `EVIDENCE {json}` line collected into
  `rea-lab/realworld/evidence-*.md`. `s3-test.sh --with-realworld FILE...` takes ANY
  file path (owner-supplied videos drop in with zero code change); a free-cloud
  `stress.yml` workflow (GitHub Actions, workflow_dispatch + weekly) runs the same
  suite at full scale.
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
