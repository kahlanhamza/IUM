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

## M7 — Speed architecture: measured profiles → the IDM brain — 🚧 **CURRENT** (owner mandate 2026-11-04; milestone 6, issues #41–#46)

M6 proved the engine's RELIABILITY promises. The owner's field measurements define the SPEED
promise: the SAME tunnel measured ~3 KB/s uplink (NapsternetV counter moment) and **1.29 Mbps
up = 161 KB/s** (Google speed test, 27.1 Mbps down / 50 ms) within one hour — a 54× swing —
so the engine must **measure the link and choose lane + fleet from measurement**, and never
pay a wall that kills healthy flows. The analysis pass also fixed the honest vocabulary:
no algorithm beats the sum of the summoned bearers; speed engineering reaches the bound
(saturation, zero waste, no false walls) and aggregates bearers (parallel flows / multipath).

- [ ] #41 — **link prober at probe time**: single-stream vs K-stream aggregate (~20 s) →
      classify per-flow cap / total-bearer cap / fat pipe; lane + fleet decision journaled
      (ADR-011 channel extension) and enforced every drive round
- [ ] #42 — **tus total→idle request timeout**: the 600 s wall is structurally
      non-convergent below ~8.7 KB/s (no legal piece ≥ 5 MiB fits one attempt); an
      idle-progress timeout owns SILENCE, the supervision layer owns DURATION
- [ ] #43 — **tus realign absorbs own-drift**: a timed-out PATCH's landed bytes are drift
      to absorb (bounded dance), never `Unknown→Fail` — misclassification made slow links
      terminal
- [ ] #44 — **lane-aware chunk floor via ADR-011**: tus has no protocol minimum; 64 KiB
      pieces confirm in ~30 s at the owner's bad-moment rate (S3 keeps its 5 MiB rule)
- [ ] #45 — **CI throttled ladder 3/8/50/160 KB/s**: the KB/s tail was discovered by the
      owner in the field because no drill went below the wall — pinned forever after
- [ ] #46 — **research track, measured spikes**: multipath aggregation (MPTCP RFC 8684 /
      part→path scheduling — S3 multipart maps 1:1 onto multiple paths) + FASP-style
      UDP+FEC for the lossy-tunnel regime; verdicts recorded, including "not worth it"

DoD: the prober's decision is journaled and enforced; the tus lane converges at 3 KB/s;
the ladder drill is green in stress.yml; every speed claim in this file cites a
measurement. The old next tranche (Sprint-2 UX / WebDAV+Drive / OAuth / v0.1.0) follows M7.
