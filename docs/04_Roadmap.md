# IUM — Roadmap

> Owned by STATUS.md §4 (current pointer). Each milestone has explicit acceptance criteria;
> a milestone is done only when every box is checked and pushed.

## M0 — Environment & scaffold ✅
Repo, persistent sandbox toolchain (`.toolchain/` + `scripts/rust-env.sh`), push helpers,
`core.fileMode false`, workspace with `ium-core`.

## M1 — Core correctness ✅ (Sprint 0, commit `145c983` + 2026-11-02 gate fixes)
- [x] 7 audited defects fixed (migrations, transactional journal, loud writes, re-rentable
      partial confirms, counted retries, journal-truth transitions, streaming adapter)
- [x] Loopback HTTP end-to-end tests over real TCP (4)
- [x] Crash recovery, retry exhaustion, stale-lease reclaim, concurrent workers, eviction
- [x] fmt + clippy `-D warnings` + 54/54 tests green locally and on 3-OS CI matrix

## M2 — Hardening & honesty ✅ (2026-11-02 — all 5 issues closed, milestone 0 open / 5 closed)
- [x] `TransferAdapter::verify` is a required trait method — verify-before-declare cannot be
      silently inherited as a default "pass" (#1, trybuild compile-fail contract)
- [x] Per-adapter verify strategy documented (HTTP: HEAD Content-Length + ETag; S3: ETag/
      checksum → ADR-010 proposal in docs/07 §4) and implemented where cheap
- [x] `replace_chunks` DELETE checks rows-affected (loud-failure symmetry) (#2)
- [x] Nightly CI: chunk-pool invariant fuzz with expanded rounds + coverage report (#4)
- [x] `MockAdapter` logs at entry (parity with the ScriptedAdapter fix) (#3)
- [x] Bonus from the #2 design note: planless-job invariant pinned (#5) — plan(0) → [(0,0)],
      EmptyPlan rejections, 0-byte e2e
- DoD met: gates green on 3 OS; every fix landed with the test that failed first.

## M3 — Real transports (current — spike done, see docs/07)
- [x] S3 multipart adapter, tested against a real server in CI — one pinned binary per OS
      runner (MinIO premise dead — archived 2026-04-25; SeaweedFS 4.51 pinned in #9;
      services containers are Linux-only; spike doc docs/07 §6) — issues #6–#9
- [x] TUS 1.0.0 adapter against a real test server (tusd v2.10.1 pinned, 3-OS) — issue #10
- [x] Resume across process restart for both (kill -9 drill in CI on Linux for S3 via #9;
      TUS byte-level restart drill in-suite via #10, subprocess drill tracked as #11)
- DoD: end-to-end uploads of >100 MiB with forced mid-body kills; zero corruption on resume;
      honest Done only after adapter-verified completion.

## M4 — App shell (Tauri 2 + React) — ✅ **COMPLETE** (2026-11-02: all 4 issues closed)
- [x] #13 — adapter-declared `max_parallel` (TUS ⇒ 1 worker) — correctness prerequisite
      landed before any UI can create a broken TUS job (ADR-012)
- [x] #14 — unfreeze: `ium-app/src-tauri` joins the workspace; CI 3-OS `ium-app` job
      (npm lane + shell clippy/tests native per runner); ADR-006 amended; icons committed
- [x] #15 — runtime wiring: `Supervisor` (drive/pause/resume/cancel), `job_updated`
      events, honest `Canceled` terminal state (+ silent-corruption parse fallback removed)
- [x] #16 — Sprint-1 UI slice (spec §8): create form (dialog plugin), jobs table with
      live progress + honest badges + engine-gated actions, per-chunk inspector; pure
      `logic.ts` under vitest (9/9), wired into the gate + CI

Unlocks only after M2 + M3. pnpm→npm already fixed in `tauri.conf.json`.

## M5 — API freeze, docs & real-data evidence — ✅ **COMPLETE** (2026-11-02: all 4 issues closed)
Owner mandate folded in: every claim is tested on REAL data (a real video file; a genuinely
throttled slow connection) — see docs/05 "Real data only".
- [x] #17 — real-data e2e drill: a real H.264/AAC video through HTTP + TUS + S3,
      byte-verified (sha256 + ffprobe on source AND received), engine-driven
- [x] #18 — real slow-connection drill: throttled socket, eviction fires on live wire
      throughput (spec §4.2 proven on real evidence, not simulated wedges)
- [x] #19 — `ium-core` public API reviewed and documented (`missing_docs` enforced,
      `cargo doc` clean)
- [x] #20 — versioning policy (semver), CHANGELOG.md discipline

## M6 — Production readiness: the probe generation — ✅ **COMPLETE** (2026-11-04: all 4 issues closed, CI run #75 8/8 green)
The engine is feature-complete and reliability-hardened — but on TWO pinned servers, ONE
impairment axis (bandwidth), minute-scale horizons, and fast CI links. Motivated by the
boundary register (docs/08 §5): each probe class exercises seams for the FIRST time.
- [x] #32 — real-network impairment: packet loss, latency/jitter, mid-body RST
      (proxy upgrade; eviction/retry seams at risk — lossy-but-alive must not be falsely evicted) — ✅ closed 2026-11-03: loss/RTT/RST models; local + cloud 41654248875 green; zero false evictions
- [x] #33 — server diversity: a third S3 implementation (`rclone serve s3`) + a second TUS
      lane if pinnable; every wire-fact delta pinned or honestly recorded (optional
      owner-cred lane for real AWS/B2 smoke) — ✅ closed 2026-11-03: rclone 1.75.2 23/23 + @tus/server 2.4.7 12/12, wire-fact ledgers; the first rclone run caught latent defect #36
- [x] #34 — long horizon & resource honesty: overnight B/s leg, journal growth + RSS
      bounds, ENOSPC-while-journaling drill (what happens when TRUTH cannot be written) — ✅ closed 2026-11-03: atomic state+event transitions, EFBIG drill (RLIMIT_FSIZE), soak bounds pinned, longhaul live green
- [x] #35 — the real shell driven end-to-end: tauri-driver e2e on the ubuntu lane
      (create → progress → pause/resume → Done; cancel → no server object) — ✅ closed 2026-11-04: both legs green on CI run #75 (tauri-driver → WebKitGTK under xvfb)

DoD: every probe closed with wire evidence; the readiness verdict in STATUS.md upgraded
per axis; any gap found files its own issue through the register discipline.

**Verdict (2026-11-04):** every probe closed with wire/cloud evidence; the gaps the probes
found (#36 Content-Length, #39 main-thread spawn abort, #40 missing ACL capabilities) were
filed and closed through the register discipline; the red streak #62–#74 ended with run
#75 all-green (8/8 jobs). Next tranche (transports / OAuth / Sprint-2 UX / v0.1.0
packaging) is the owner's call.

## M7 — Speed architecture: measured profiles → the IDM brain — ✅ **COMPLETE** (2026-11-04: all 6 issues closed with measured evidence + the drill-discovered #47 fixed via the full loop; CI runs #80/#81 green, real-tusd wire 12/12)

M6 proved the engine's RELIABILITY promises. The owner's field measurements define the SPEED
promise: the SAME tunnel measured ~3 KB/s uplink (NapsternetV counter moment) and **1.29 Mbps
up = 161 KB/s** (Google speed test, 27.1 Mbps down / 50 ms) within one hour — a 54× swing —
so the engine must **measure the link and choose lane + fleet from measurement**, and never
pay a wall that kills healthy flows. The analysis pass also fixed the honest vocabulary:
no algorithm beats the sum of the summoned bearers; speed engineering reaches the bound
(saturation, zero waste, no false walls) and aggregates bearers (parallel flows / multipath).

- [x] #41 — **link prober at probe time**: single-stream vs K-stream aggregate (~20 s) →
      classify per-flow cap / total-bearer cap / fat pipe; lane + fleet decision journaled
      (ADR-011 channel extension) and enforced every drive round — drills prove BOTH
      classifications (per-flow SK/S1 = 4.0× → fleet 4; bearer 1.0× → fleet 1)
- [x] #42 — **tus total→idle request timeout**: the 600 s wall is GONE;
      `read_timeout(idle)` is the progress tripwire (30 s default); MEASURED finding: a
      moved()-watchdog cannot work — kernel/hyper buffers absorb MiB before backpressure
- [x] #43 — **tus realign absorbs own-drift**: bounded dance (≤4 rounds) with the ENTRY
      HEAD as the alignment point (the CI-run-78 post-mortem: the journal can be BEHIND
      after crash recovery; exact `moved−wasted` reckoning, the doomed blind PATCH gone)
- [x] #44 — **lane-aware chunk floor via ADR-011**: tus declares 64 KiB; the planner obeys
      `declared_lane_floor` clamped [1 KiB, CHUNK_MIN]; en route: **423 Locked is a WAIT,
      not a failure** (`ErrorKind::Busy`, uncharged/un-evicted) and beyond-lease resplit
      drift confirms-in-full on a single-writer lane
- [x] #45 — **CI throttled ladder 3/8/50/160 KB/s**: 99%+ of wire at every rung
      (3070.5/8186/51081/162884 B/s achieved), byte-exact, overhead MEASURED
      0.72% → 0.05%; wired into stress.yml (`IUM_THROTTLE_LADDER=1`)
- [x] #46 — **research track, verified sources**: docs/07_Speed_Architecture.md
      (RFC 8684, draft-ietf-quic-multipath-21, RFC 3449, RFC 6330, tus concatenation) +
      ADR-012; honest bound: research REACHES the bearer sum and AGGREGATES bearers
- [x] #47 — **resplit × tus lease race** (discovered by the #44 drill, filed with full
      journal evidence, closed the same day via the full loop): the driver's never-consumed
      epoch signal poisoned every post-resplit lease and the ResplitUnderneath arm
      stranded live leases silently; fix = epoch consumed at lease time + identity guard
      in the arm + driver reclaim-then-decide with `drive_pool_snapshot` evidence on
      every non-clean exit; the drill is un-ignored and permanent — suite 159 → **160**

DoD: met and evidenced on each issue — the prober's decision is journaled and enforced;
the tus lane converges at 3 KB/s (the #45 ladder proves 3 KB/s end-to-end at ≥99% of
wire); the ladder drill is green in stress.yml; every speed claim in this file cites a
measurement. The next tranche (Sprint-2 UX / WebDAV+Drive / OAuth / v0.1.0) is the
owner's call.
