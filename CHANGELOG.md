# Changelog

All notable changes to IUM are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/) with the 0.x pre-1.0 rules in
`docs/05_Engineering_Process.md` (§ Versioning policy).

Every entry cites its issue numbers — evidence lives in the tracker, not in
prose. An entry without verifiable issue numbers does not belong here.

## [Unreleased]

### Added
- The telemetry stance: `docs/11_Telemetry_Stance.md` is the WRITTEN privacy
  contract — anonymous diagnostics (terminal-state events + panic captures)
  over an allowlisted field set, opt-out consent enforced at the engine's
  single write point, and a capped local JSONL buffer the user can read and
  clear from Settings → Privacy. No transport ships in this build (the sink
  is the pinned owner ask), so nothing leaves the device by construction.
  The charter's collected-fields table is asserted equal to the code on both
  the Rust and TS sides — the stance cannot drift (#75).
- The brand sheet: `docs/10_Brand_Sheet.md` records the decided identity —
  the IUM name and wordmark, the voice (honest, specific, calm, the engine's
  truth kept visible), the naming rules (friendly product labels over
  technical seams), the color/type/iconography sourced from the tree — and
  `brand.test.ts` enforces it: the sheet cannot drift from `theme.css`'s
  tokens and the enumerated copy surface follows the voice (#73).
- The supervisor concurrency limiter: `Supervisor::with_concurrency_limit` /
  `with_handle_and_limit` admit at most N simultaneous drive tasks and
  FIFO-queue the rest; the shell boots bounded at 4 — a 100-file intake is
  no longer 100 simultaneous uploads. Legacy constructors stay unbounded
  (#72).
- The 100-jobs load e2e: 100 interleaved jobs over one manager and one
  journal reach the exact honest terminal mix (92 Done / 8 Failed) with
  journaled retry receipts and bounded fd/RSS orchestration (#72).
- The installer-size budget gate: `scripts/installer_budget.json` pins the
  v0.1.1 measured baselines and per-format max sizes (NSIS 10 / MSI 12 /
  DMG 12 / deb 12 / rpm 12 / AppImage 110 MB); `scripts/installer_size_gate.py`
  fails any installer over budget or without a decided budget, enforced
  per-OS in release.yml and self-tested in ci.yml (#72).

## [0.1.1] — 2026-11-06

The product bar cut: the shipping bar of #65 plus product-bar lanes 1–4 of #66 —
design-system depth, the credentials vault, the error-copy table, and
supply-chain integrity (SBOM + provenance) — with the first-ever end-to-end
run of the installer pipeline against a real tag.

### Added — the shipping bar (#65, M11)

- The shipped-product UI pass: a real design system (`theme.css` — light/dark
  tokens via `prefers-color-scheme`, 4-pt spacing, 150 ms motion with
  reduced-motion respect, zero runtime dependencies), a brand header with the
  engine status pill, drag-and-drop file intake riding the same stat-and-plan
  path as paste-a-path, recent-destination suggestions from the journal,
  friendly adapter display names, structured capability badges, toast
  notifications for failed actions, a designed empty state, and an animated
  per-chunk inspector (#65).
- The per-OS installer pipeline: tag-triggered CI builds (Windows MSI + NSIS,
  macOS DMG, Linux deb/AppImage/rpm) with sha256 sidecars, measured sizes, a
  silent-install → boot → clean-uninstall smoke gate per OS, and a
  "The installers" release-notes section (#65).
- Zero-touch OS setup: WebView2 auto-resolves via downloadBootstrapper on
  Windows; the NSIS installer runs per-user (no UAC); no PATH edits, no
  service registration, journal data survives reinstalls by design (#65).

### Changed

- The UI's visual layer moved from inline styles to the token system; the
  app-e2e seams are untouched (element ids, registry-name option values,
  semantic table structure, `[data-state]` chips) (#65).
- The design system became typed component primitives (`Button`, `TextInput`,
  `Select`, `Field`, `Chip`, `Badge` in `components/ui.tsx`) composing the same
  token classes — bundle and visuals unchanged (#66, #67).
- Accessibility is a CI gate, not a hope: axe-core audits every screen state
  and every primitive on `npm test`; landed the two real defects it caught
  (unnamed progress bars announcing bare numbers; a `role="dialog"` on
  `<aside>` that strict assistive tech would drop) (#66, #67).

- The credentials vault: `.secrets/` env-contract as a product surface — a
  0600 atomic store in the OS data dir, env-wins precedence, secrets never
  leaving the process, settings dialog axe-audited; feishu and the 74-backend
  rclone bridge in the adapter registry (#66, #69).
- The error-copy table: every engine error kind becomes a human sentence with
  an action hint, raw engine truth kept visible; the copy lives in one table,
  not scattered literals (#66, #70).
- Supply-chain integrity on every release (#66 lane 4, #71): two CycloneDX
  SBOMs (the Rust workspace closure via checksummed syft over the root
  `Cargo.lock`; the app npm tree including devDependencies, which ship in the
  bundle), SLSA v1 build provenance on every shipped artifact (GitHub
  artifact attestations over OIDC), sigstore bundles attached for offline
  verification, and an in-job attestation verification gate — a release
  cannot exist unless its attestations verify. All workflow actions
  SHA-pinned; least-privilege permissions per job.
- A supply-chain check tool, `scripts/release_supply_chain_check.sh`: pure
  REST verification that a release carries both SBOMs and that every
  installer resolves to a live attestation.

### Fixed — what the first pipeline rehearsal caught (#71)

- The updater signing lane was unshippable in silence: `createUpdaterArtifacts`
  + pubkey had landed but the minisign private half existed nowhere (the
  installer pipeline had never run against a real tag). Key rotated
  (zero clients ever held the old one — no release shipped an updater
  artifact); the private half lives in the repo secret store only.
- The rehearsal also pinned two packaging wire facts: MSI requires
  numeric-only prerelease identifiers, and RPM forbids `-` in the version
  string — the three package formats intersect only on dash-free versions.

## [0.1.0] — 2026-11-05

The first frozen public surface (per #19): `JobManager`, `Supervisor`, `JobSpec`,
`JobRecord`, `JobSnapshot`, `SessionJournal`, the `TransferAdapter` contract,
the three transport adapters, `ChunkPool`/`Lease`/`ChunkState`, `EvictionPolicy`,
`RetryPolicy`, `JobState`, `EngineError`.

This cut closes M1→M9 (the frozen facade grew the speed architecture —
measured profiles, the adaptive fleet, bonding, FEC, compression — and new
transport lanes: Feishu Drive and the rclone bridge over 74 backends) and
lands the first M10 rung. 258 core tests green on the tagged tree; the
guarantees are the loop's measured ones: verify-before-declare (ADR-010),
zero-re-upload resume on every lane, the journal as the only truth (ADR-002).

### Added — engine (M1–M2, Sprint 0)

- M2 hardening & honesty (#1–#5): `verify` is a REQUIRED trait method (#1) —
  the seed of ADR-010's verify-as-commit-gate; journal rows-affected checks on
  `replace_chunks` DELETE (#2); MockAdapter call-log observability parity
  (#3); the nightly chunk-pool fuzz + coverage lane (#4); the planless-jobs
  invariant pinned — every committed job has ≥1 chunk (#5).

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

### Added — the stress-evidence tranche (2026-11-03, between M5 and M6)

- #27 — adaptive resplit: pattern #9 is now driven from measured throughput.
  The pool's `resplit_pending` and `suggested_chunk_size` existed, were
  invariant-fuzz-tested, and had ZERO runtime callers — planning was one-shot
  per attempt, so on the owner's 32 KiB/s-class reality a planned 22 MiB part
  confirmed its first byte after ~11 s (S3 lanes: the 5 MiB protocol floor
  bounds the best case) and nothing adapted. Now the drive loop's window-close
  path consults a pure, shrink-only `ResplitPolicy` (fleet median bps in,
  target-or-None out; ~30 s of wire time per part at the observed median,
  90 s slow-confirm default, 25% deadband, and a would-actually-split gate
  that also pinned a REAL resplit helper defect: at the floor regime the split
  loop dropped every sub-floor piece and silently stayed whole — 12.4 MiB at a
  5 MiB target split into nothing). The single-flight winner retires leases
  (peers cancel their in-flight PUTs via an epoch watch BEFORE the pool is
  reshaped), persists the whole plan transactionally (`replace_chunks`),
  journals `chunk_resplit` with its real inputs, and restarts retry budgets
  (the plan they keyed on no longer exists). Honest accounting under the
  reshuffle: a confirm/error arriving for a retired lease is verified against
  the lease's byte range under the same pool lock and dropped with a journaled
  `stale_confirm_dropped`/`stale_error_dropped` event — never applied to a
  stranger chunk, never charged to a budget. The grow direction (merge parts
  on fast links) was drafted and deliberately rejected: S3 parts confirm
  atomically, so bigger parts mean worse feedback and abandoned prefixes for
  only marginal request savings — no measured scenario needed it. Splitting
  never creates bandwidth (N parts on one slow uplink still share it) — the
  win is first-confirmation latency, sooner eviction votes, less wire re-sent
  when a connection dies mid-part; docs say so. Pinned by
  `tests/adaptive_resplit.rs` (RED: the API was inexpressible, E0432/E0599
  at the pre-fix HEAD) + policy unit tests + the floor-regime pool test.

- #28 — the worse-case ladder, free-cloud-proven, and "the resplit must
  happen anyway": the owner's uplink is B/s-class, worse than the 32 KiB/s
  leg, so the bench now climbs 32768 → 2048 → 512 B/s. The throttle proxy
  gained the UNIFORM whole-uplink model (`--victim-part all`: EVERY
  UploadPart PUT throttled, no latch/expiry — the median collapses and the
  resplit fires once, floor-clamped to 5 MiB pieces; the victim mode's
  latched leg is outvoted by fast peers and cannot resplit — honest
  expectation stated on the issue). The `realworld_stress` driver gained
  `s6worse` (timeboxed observation legs: no Done assertion — at 512 B/s a
  30 MB upload takes ~17 h; the leg cancels at the timebox, exercising
  cancel under a dying link) and surfaces `chunk_resplit` payloads in
  `s3hostile` evidence (honestly empty in victim legs). Cloud RED
  41571828134 produced the deeper rule: on a runner with tiny buffers the
  adapter's first write never completed, the median was UNMEASURABLE, and
  nothing fired — so `ResplitPolicy::plan_frozen` now fires past
  `slow_confirm` with the JOB-level clock (hostile servers reset per-lease
  clocks with their own 30 s read deadlines) and a floor-only target with
  hysteresis: adaptation when NOTHING is measurable. Wire-accounting
  lessons landed in the harness: the proxy relays in ≤1 s granularity
  (64 KB bursts left ~32 s of upstream silence → SeaweedFS's 30 s volume
  idle timeout killed every slow PUT), and `s6worse` reports its progress
  curve to stderr. Proven in the free cloud (canonical run 41578950703 @
  `1022bb9`): 32768 B/s via the measured path (median 72,090 B/s → 5 MiB
  floor), 2048 + 512 B/s via the frozen rule (`median_bps: 0.0`).

- #29 — deterministic mid-flight cancel on fast links. S4cancel raced them:
  parallel parts confirm in one step, so the cancel driver's first sight of
  progress already sat at the verify gate, and a post-completion kill hit an
  already-Done journal. The `--allow-calls 1` wedge shape (one confirmed
  call, then a bounded hold) makes the kill land deterministically at 25%:
  S4cancel cancels in 0.079 s with server HEAD 404; the same wedge gave
  S4pause a BOUNDED 10 s plateau (`pause_latency_s: 10.04`).

### Added — M6: production readiness — the probe generation

- #32 — real-network impairment models in the stress bench: `throttle_proxy.py`
  gains `--loss PCT` (RTO-scale stalls U(0.2,2.0) s per relayed segment — the
  application-observable of packet loss; a userspace relay cannot drop
  in-stream bytes), `--latency-ms`/`--jitter-ms` (connect + response RTT,
  bandwidth untouched), and `--reset-after-bytes N` (ONE-SHOT hard RST of both
  ends when the first UploadPart PUT crosses N bytes — a real mid-body
  connection reset). New `realworld_stress s7net --model loss|rtt|reset`
  scenario asserts journal truth per model (RTT link: ZERO evictions/resplits;
  RST: ONE counted `chunk_error {kind: Transient, action: Retry}` then Done);
  harness S7 legs + `stress.yml` `net_models` input. Wire truth per leg comes
  from the proxy's LOSS/RESET-WIRE lines; server truth from read-back sha256.

- #33 — server diversity: the suites now run against THREE S3
  implementations and TWO TUS implementations. `scripts/s3-test.sh --server
  rclone` boots pinned rclone 1.75.2 `serve s3` (per-release SHA256SUMS is
  the supply-chain guard — rclone ships no per-asset checksums); the full
  23-test suite is green on it, re-confirming on a third implementation:
  ListParts IS implemented although rclone's docs omit it, and Go's XML
  encoder emits `&#34;` numeric entities in ETags (the exact #9 quirk class).
  `scripts/tus-test.sh --server tus-node` boots pinned @tus/server 2.4.7 +
  @tus/file-store 2.1.2 (committed package-lock.json — registry integrity
  hashes are the supply-chain guard); the suite is green there with ONE
  loud, recorded delta: tusd's GET-download is its own extension, NOT tus
  1.0.0 core — @tus/server answers GET on the upload resource with 404, so
  the read-back tests skip loudly on that lane (`IUM_TUS_SERVER` tells the
  tests which wire they are on). CI gains Linux diversity legs; the kill-9
  drills and the real-data drill stay canonical on SeaweedFS/tusd.

- #34 — the journal seam (M6): `set_job_state_event` is ATOMIC — a state and
  its event commit together, the half-transition RED closed; honest Paused
  park on mid-transfer ENOSPC; real-OS EFBIG pinned via RLIMIT_FSIZE; soak
  bounds pinned (16,391 events → 2.4 MB, WAL→0, RSS +5.5 MiB); the longhaul
  leg runs vs SeaweedFS with honest timebox-pause and fresh-journal replay
  byte-exact.

- #35 — real-shell WebDriver e2e (M6): the `ium-e2e` crate drives the
  PACKAGED shell (WebKitGTK under xvfb) through create → live progress →
  inspector — the UI's truth asserted on the real app, 3-OS CI leg; later
  hardened by #37's tus leg.

### Added — M7: speed architecture — measured profiles → the IDM brain

- #41 — the link prober (M7): S1 canary + K independent sessions (canary/K —
  same wall time both phases), classify PerFlowCap (SK ≥ 2×S1) / BearerCap
  (<1.5×) / Mixed; the piece = 20 s of measured wire clamped lane-floor..max;
  drills prove both shapes (per-flow SK/S1 = 4.0× → fleet 4; bearer 1.0× →
  fleet 1). Wired into the managed flow by #48.

- #42 — the 600 s wall GONE (M7): no total request timeout; the progress
  tripwire is `read_timeout(idle)` (30 s default, `with_idle` compresses for
  drills) — a stalled link fails loudly, a slow-but-moving link runs forever;
  the RED leg measured at 2 KB/s, and the moved()-watchdog idea was proven
  dead en route (kernel/hyper buffers absorb MiB before backpressure).

- #43 — the realign dance is a bounded loop (M7): ≤4 rounds, beyond →
  Transient; Unknown was reserved and then REMOVED entirely when #44 showed
  beyond-lease drift is our own resplit; waste reckoning settled once per
  call; the run-#78 CI post-mortem added the ENTRY HEAD as the alignment
  point — exact moved−wasted, the doomed blind PATCH gone.

- #44 — the lane-aware chunk floor (M7): `ProbeReport.min_chunk_bytes` rides
  ADR-011 — tus declares 64 KiB, the planner (plan/resplit/cap-replan) obeys
  `declared_lane_floor` clamped [1 KiB, CHUNK_MIN], so pieces confirm in ~30 s
  at the owner's bad-moment rate; the drill forced two real fixes:
  resplit-induced beyond-lease drift confirms in full (single-writer lane) and
  423 Locked is a WAIT not a failure (`ErrorKind::Busy` — uncharged,
  un-evicted).

- #45 — the throttled CI ladder 3/8/50/160 KB/s (M7): 99%+ of wire at every
  rung through the real engine (3070.5 / 8186 / 51081 / 162884 B/s achieved),
  byte-exact, overhead MEASURED 0.72%→0.05%; wired into `stress.yml`
  (`IUM_THROTTLE_LADDER=1`), the PR lane keeps compressed drills — the tail
  was found in the field because no drill went below the wall; never again.

- #46 — the research track (M7): `docs/07_Speed_Architecture.md` on verified
  sources (RFC 8684, the QUIC multipath draft, RFC 3449, RFC 6330, tus
  concatenation) with the owner's research brief ingested verbatim; the
  honest bound — nothing beats the sum of the summoned bearers; research
  REACHES the bound and AGGREGATES bearers (landed as #51 and #52).

### Added — M8: absolute speed — the prober drives, the fleet adapts, the lanes multiply

- #48 — the link prober, WIRED (M8 rung 1): `probe_link` (#41) had zero
  production callers — the engine never measured, never journaled
  `link_profile`, never obeyed a recommendation. Now a managed tus job with
  `IUM_LINK_PROBE=1` measures its link at the probe boundary (paced canaries,
  real payload bytes, ~10 s of wire per phase), journals the profile exactly
  once, and the planner OBEYS the measured piece (`replan_measured`, shrink
  AND grow — plan phase, both directions legal; composed with the adapter's
  per-call cap, ADR-011 wins). Opt-in by env: every byte is sacred on a
  metered bearer; a failed probe is a lost measurement (`link_profile_skipped`),
  never a lost job.

- #49 — AIMD fleet control (M8 rung 2): the drive waterline is SEEDED from
  the journal (last `fleet_adjusted` | measured `recommended_fleet` | ceiling,
  clamped to the job's pending count) and ADAPTED live — a driver-side ticker
  folds each second's completed-call evidence (server-confirmed bytes, counted
  errors) into a pure `AdaptiveFleet::step`: multiplicative decrease (×0.7) on
  an error rate > 0.3, additive increase (+2) while goodput clears the best by
  8%, −1 on a >10% regression, hard hold at the plateau, feedback gates
  (≥ 64 KiB AND ≥ 0.5 s for goodput verdicts, ≥ 4 calls for error verdicts).
  Workers above the waterline PARK at their lease boundary and wake on growth
  — both directions bind MID-round; every decision is journaled
  (`fleet_adjusted`) so a fresh manager resumes exactly there (ADR-002). The
  54×-in-an-hour link swing is finally answered in BOTH directions: shrink
  was eviction/resplit's job, growth had no owner until now.

- #50 — the tus concatenation lane (M8 rung 3): the server declares, the
  lane parallelizes. `probe_fleet(spec, seed)` (new trait method, default =
  `probe`) discovers `Tus-Extension: concatenation` at probe time (a failed
  discovery degrades to the classic lane — a job never dies on an optional
  probe), creates N partial sessions (`partial *i*` owns
  `[i·L, min((i+1)·L, total))`, clamped to ceil(total/64 KiB)), and declares
  `max_parallel: Some(N)` on the #13 channel — the prober (#48) and the AIMD
  fleet (#49) adapt WITHIN it. A straddling chunk is two segments; a
  fresh-partial straddle pushes the neighbor's bytes from the server's
  offset (absolute source mapping `base + local`); confirm counts only the
  segment's span. `finalize(spec)` materializes the final at the
  all-Confirmed boundary (`concat_final` intent journaled BEFORE the call,
  `done` + final URL AFTER — tusd re-POSTs create a SECOND final; the
  journaled URL is never re-POSTed); verify gates on the FINAL URL and
  honestly reports "not finalized" before it. 10/10 wire tests live vs tusd
  2.10.1 (`scripts/tus-test.sh --concat`); @tus/server has no concatenation
  (the fallback leg runs only there).

- #55 — the aggregate-vs-single measurement drill (the M8 acceptance item
  of #50): `scripts/tus-test.sh --measure` drives the REAL managed job
  through `scripts/throttle_proxy.py` shaped wires — per-connection cap
  (`--per-conn-rate`), shared bearer (`--total-rate`), and a seed-1 control.
  On the record (`tests/concat_measurement.rs`, env-gated
  `IUM_CONCAT_MEASURE=1`, skips loud on the PR lane): per-flow-capped wire →
  the lane declares 4 partials and measures **3.71×S1** (ideal 4.00×; the
  gap is the drill's own wall-clock arithmetic); shared-bearer wire → the
  prober recommends fleet 1, the waterline parks the fleet, **0.82×S1** (no
  multiplication, byte-exact); control → **0.97×S1** (the instrument's
  honesty baseline, RED-proven — the multiplication assertion fails there).
  Wire facts from the live rehearsal (`scripts/issue55_wire_rehearsal.sh`):
  tusd honors the request Host (Location points at the proxy — the shaped
  wire carries the whole session); no-body requests ride free. Three
  instrument bugs caught by the drills themselves: the bearer bucket's
  idle-credit burst (ceiling now 250 ms), read-schedule pacing letting a
  socket-buffer-sized body cross at 4× its cap (the relay now pays its
  schedule debt before returning), and a silent death on an upstream
  early-409 (now `UPSTREAM-GONE` + honest teardown).

- #51 — the paths lane (M8 rung 4, the chunk pool sums independent bearers):
  `netpath` — NetPath = Default/SourceAddr/BindDevice, reqwest-native
  bindings (`interface()` = SO_BINDTODEVICE/IP_BOUND_IF, cfg-gated, Windows
  loud-skip; source-addr dual-path needs NO privilege); discovery and
  degradation journaled (`path_plan`/`path_excluded` at the probe boundary,
  `IUM_PATHS`), weighted round-robin + per-path AIMD pure-fn tested, per-path
  prober, fleet seed composes live-paths × per-path fleet, bonded concat
  lane; the `tus-test.sh --multipath` drill: single 57.7 vs dual 98.7 KiB/s =
  1.71× ≥ the 1.5× bar, byte-exact both legs; no `IUM_PATHS` = pre-#51
  byte-for-byte.

- #52 — the FEC lane (M8 rung 5): `fec` pure module (RFC 6330, crate raptorq
  1.8.1, wire facts W1–W13 rehearsed live in `scripts/wirefacts52/`, pinned
  vectors in the contract tests) + `adapter_fec::FecAdapter` — the client
  half of relay mode: the group grid derives from source size ALONE
  (resplit-stable by construction), per-group lane instances, eager sessions
  at probe with the `fec_plan` journaled; any K of (K+R) rebuilds byte-exact
  — proven LIVE end-to-end: R erasures per group on the RECEIVED streams
  after a managed job → source exact; wire 1.2503× at every loss rung
  (0/3/5%) with the honest verdict: parity wins only on an erasure channel.

- #53 — the entropy-gated zstd lane (M8 rung 6, the Cause-D byte lever):
  `compress` module (entropy gate = head/mid/tail 64 KiB samples at fast
  level 3, bar 1.15, ratio honesty pinned; streaming checksummed zstd encode;
  decode+hash integrity primitive; statvfs disk check) +
  `adapter_compress::CompressAdapter` — single-chunk shape: the ARCHIVE is
  the wrapped lane's source, so kill-9 / zero-re-upload / 409-realign /
  verify are inherited structurally; the manifest (mode/level/both
  hashes/sizes/ratio) rides the probe event; decline paths = plain jobs with
  the reason journaled (`IUM_NO_COMPRESS` kill-switch); live drill: 86.6%
  saved (4 MiB → 562 KB), video-like bytes declined in 0.03 s.

### Added — M9–M10: new transports

- #57 — the prober covers the S3 lane (M9 rung 2): the measured-profile
  architecture is now LANE-WIDE. The adapter owns its wire, so the adapter
  owns its probe — `TransferAdapter::probe_link` (ADR-011's shape extended
  from #50's `probe_fleet`; default `Ok(None)` = pre-#57 silence, no
  adapter-name string matching in the host). The tus lane delegates to the
  unchanged free-standing prober; the S3 lane measures multipart
  transactions (create → one canary part → K concurrent parts on the SAME
  upload → ABORT — never complete: the 5 MiB floor is server-dependent,
  gofakes3 accepts small-part completes while SeaweedFS rejects them with
  EntityTooSmall, and the abort is the no-orphan cleanup on both pinned
  servers); the rclone bridge (#63) inherits the prober through its inner
  lane. Classification is ONE shared pure fn (`prober::classify`) — the
  same ≥2×/<1.5× arithmetic both legs pin. The S3 lane's 5 MiB multipart
  floor applies DOWNSTREAM in the pool planner (the profile is DATA, not
  policy — an aggressive recommendation is floored structurally); the #51
  path plan keeps its tus-wire gate. Journal contract unchanged:
  `link_profile` once, `link_profile_skipped` on failure, planner obeys
  `replan_measured`. Live drill (`IUM_S3_PROBE_DRILL=1`): per-conn 64 KiB/s
  classifies PerFlowCap (fleet 4), a shared 64 KiB/s bearer classifies
  BearerCap (fleet 1), and every leg leaves the bucket's
  ListMultipartUploads EMPTY — the probe aborts its own upload.

- #63 — the rclone bridge lane (M10 rung 1): one adapter = 74 backends day
  one (`RcloneBridgeAdapter` owns/attaches `rclone serve s3` and delegates
  every transfer to the proven `S3MultipartAdapter` — SigV4, ETag
  journaling, the ADR-010 verify gate all inherited). Zero credentials
  needed to run: anonymous serve bound to 127.0.0.1 (the throwaway-SigV4
  workaround for the upstream `--auth-key` defect), env-injected remotes,
  spawned serve reaped on drop. Byte-level resume across bridge restarts
  from the journaled token (the kill-9 shape, drilled live).

- #56 — the Feishu Drive lane (M9 rung 1): `adapter_feishu` over the
  drive-v1 resumable multipart API (the owner's own storage; tenant
  credentials from env, no secrets in the repo). Wire facts rehearsed
  LIVE (W0–W11, probed 2026-11-05): platform-fixed 4 MiB blocks
  (`block_num = ceil(size/4 MiB)`, 24 h transaction), parts as
  multipart/form-data with the binary field named `file` and an Adler-32
  checksum, out-of-order parts accepted, even_split shapes REJECTED
  (1062009) — hence the new prescribed-geometry channel:
  `ProbeReport::uniform_block` + the trait twin, `ChunkPool::
  plan_uniform_tail`/`new_uniform` (remainder concentrated in the tail),
  and the manager refusing every reshape (resplit, cap replans) on such
  lanes. Session = the upload transaction (`upload_id` journaled as the
  JSON session token; `reattach` restores; the manager's finalize-once
  machinery (#50) journals `upload_finish` and its crash-window repair).
  Verify = streamed download + sha256 (the wire exposes NO size metadata —
  W4). Error mapping is loud: 1061021 → ExpiredSession, 1061045 → Busy,
  1062009 → Corruption, 1061073 → AuthRequired. The env-gated live drill
  (`feishu_drive_live_drill`): 9 MiB / 3 blocks, out-of-order parts,
  journal-truth resume on a FRESH adapter, finish, byte-exact verify,
  cleanup — green in 11 s with vault credentials. Suite 226 → 236.

### Fixed

- #47 — resplit × in-flight tus lease: the drive loop stranded a `Leased` row
  with no owner and no journaled resolution, then walked to Finishing and the
  verify gate failed the job (`verify_inconsistent` — honest, but the job was
  lost). Two compounding defects: the driver's `resplit_rx` never consumed the
  epoch change and a receiver clone preserves the unseen marker, so after any
  resplit EVERY lease taken in a later round died instantly on its first
  `select!` poll (a stale epoch signal — the RED drill's piece 0 died this way
  twice); and the `Step::ResplitUnderneath` arm journaled nothing, which is
  correct only for leases the winner's rewrite retired. Three layers: the
  worker marks the current epoch seen before leasing (`borrow_and_update`);
  the `ResplitUnderneath` arm runs the same identity guard as the confirm/
  error/eviction arms (a live lease returns to the pool journal-first under
  the pool lock; a retired identity stays unjournaled); and the driver now
  reclaims ownerless `Leased` rows at ROUND END before any exit decision
  (reclaim-then-decide — `pending == 0` is unreachable while a reclaimable
  lease exists), bounded by `MAX_IDLE_RECLAIM_ROUNDS` so a deterministic
  strand bug fails loudly instead of livelocking, and every break-to-Finishing
  exit that leaves non-uploaded bytes journals a full `drive_pool_snapshot`
  ahead of the verify gate. RED: the #44 lane-floor drill un-ignored
  (`slow_tus_lane_resplits_below_the_s3_minimum`) — Failed at HEAD `cbc578d`;
  GREEN: Done, byte-exact, 3/3 stable runs; suite 159 → 160 core tests.

- #39 — the shell aborted on its first-ever Upload click: `create_job` and
  `resume_signal` are SYNC tauri commands (tauri v2 runs them on the GTK
  main thread, per docs) and `Supervisor::drive` spawned with a raw
  `tokio::spawn`, which panics without a runtime context; the unwind
  crossed the webview IPC FFI boundary and aborted the whole process
  ("Session terminated without a reply", CI runs #65–#70). The supervisor
  now captures a `tokio::runtime::Handle` at construction
  (`Supervisor::with_handle` for context-less hosts) and spawns through it
  from any thread; the tauri setup passes the async-runtime handle. Pinned
  by `tests/supervisor.rs::drive_is_callable_off_the_runtime_thread` and
  `with_handle_constructs_off_runtime_and_drives_off_runtime` (RED panic
  captured at supervisor.rs:77 pre-fix).

- #40 — the webview had NO capabilities file, so the ACL granted it
  nothing: app commands are allowed by default (tauri docs) but the
  `job_updated` event stream (`plugin:event|listen`) is permission-gated —
  the UI silently never received a snapshot and froze on its creation-time
  row while the engine uploaded (e2e run #71: row stuck at `probing pct=0`
  while the recorder recorded the full 24 MiB). `capabilities/default.json`
  now grants `core:default` (includes `core:event:default`) and
  `dialog:default` (the file picker — the paste-a-path seam had been
  dodging it).

- #36 — zero-byte PutObject sent no Content-Length: hyper puts neither
  Content-Length nor chunked-TE on the wire for an empty fixed-size body,
  and AWS S3 semantics (which rclone/gofakes3 enforces with 411
  MissingContentLength where SeaweedFS happened to be lenient) treat a
  PutObject without a length as invalid — found by #33's first rclone run
  (22/23) and fixed the same way UploadPart already does it: an explicit
  `content-length: 0` extra header. Both S3 lanes green after the fix.

- #30 — the kill-9 TUS drill's wire ceiling was a calibrated race, now the
  zero-re-upload invariant is EXACT. Nightly run 41618615182 failed with
  `resume moved 21168128 for 20971520 missing — beyond the 64 KiB slack`: the
  rejected 409 PATCH's socket flush (frames hyper pulls before `send()`
  resolves) is a client↔server scheduling race — 0–48 KiB on a quiet machine,
  192 KiB on a loaded runner — and the drill had pinned that timing artifact
  at 64 KiB (the #10 finding's 16–24 KiB, one quiet day). Investigation
  (A/B worktrees at pre-#27/pre-#28 + the new `scripts/dbg-tus-drill.sh`
  journal-capturing driver) exonerated the manager completely: the resume
  event log shows exactly `SessionLost → Reattach → realign confirm`, no
  resplit/stale/retry involvement, and the flush distributions are identical
  across the whole regression window. Protocol rehearsal: tusd consumes
  NOTHING of a rejected body (offset immutable), and `Expect: 100-continue`
  was REJECTED by rehearsal — reqwest 0.12/hyper 1.12 neither holds the body
  on conflict (8192 pulled, same as the control) nor survives the happy path
  (`BodyWriteAborted, NotEof`). Fix: measure the waste instead of bounding it
  silently — `ProgressReporter::waste()/wasted()` reclassifies the rejected
  attempt's flush (`moved()` semantics untouched: eviction/resplit keep
  counting real wire bytes); `TusAdapter::upload_chunk` reclassifies at the
  Conflict branch before the realign dance; the drill child prints `WASTED`
  and the parent asserts `WIRE − WASTED == SIZE − SERVER_OFFSET` exactly —
  every byte the server lacked crossed once, every wasted byte accounted,
  and a manager that re-sends server-held bytes still fails loudly (re-sent
  server-held bytes are not waste-reclassifiable). The offline realign test
  gained the exact `moved() − wasted() == tail` claim (RED: E0599
  pre-fix). Also adds `scripts/dbg-tus-drill.sh` (hand-driven drill with the
  journal captured — the instrument that closed this).

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

- #37 — the CI-debt audit (REOPENED by the owner — runs #62–#69 all failed
  while new pushes kept landing): app-e2e leg A ran `http_oneshot`, which
  plans ONE chunk — the honest Σ-confirmed percent cannot move mid-transfer,
  so the progress assert was unsatisfiable BY CONSTRUCTION (runs #65–#69
  failed identically: a design flaw, not a flake). Fix: the loopback recorder
  gains a real TUS lane (12/12 of the adapter's own wire suite passes against
  it), leg A selects tus with 6 connections, the assert polls with a 45 s
  deadline; flake faces verified dead (`stress_tus_flakes.sh` 36/36).

- #54 — the pause drill stopped betting on clocks: run #93's Windows-only
  red was a drill-side flake (a pauser slept 5 ms and unwrapped — Windows
  ~15.6 ms ticks plus runner starvation let the job reach Done first, and
  `pause()` honestly refused); fix = the mock's drill wedge (`arm_wedge(0)`,
  the pauser polls `wedge_engaged`) makes mid-flight pause DETERMINISTIC;
  ZERO production code involved (identical code green in #90–#92); rerun
  green, flake proven on the record.

### Changed

- CI: 3-OS matrix (ubuntu/macos/windows) for engine and shell; nightly fuzz
  (env-gated, mutation-proven) + coverage artifact + kill-9 drills (#4).

### Process

- Issue-driven loop with failing-test-first (docs/05); `scripts/gate.sh` gates the
  COMMITTED tree in a detached worktree after defect #11 (a push that never compiled);
  STATUS.md as the single source of truth (ADR-008).

- #21 — CI forensics + engine re-proof on the owner's question ("why did CI
  fail — does the engine really work? upload must be fast even on slow
  connection"): all 40 runs audited, 4 failures root-caused, the engine
  re-proven on the record.

- #31 — the boundary register (`docs/08_Boundary_Register.md`): the entity/
  edge map + seam table — what breaks what, pinned by which test; a
  checkable repo artifact, not chat prose.

- #38 — README to product standard: badges (ci/nightly/stress), highlights,
  honest scope; roadmap truth synced.
