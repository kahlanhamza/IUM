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

## M6 — Production readiness: the probe generation (filed 2026-11-03, milestone 5)
The engine is feature-complete and reliability-hardened — but on TWO pinned servers, ONE
impairment axis (bandwidth), minute-scale horizons, and fast CI links. Motivated by the
boundary register (docs/08 §5): each probe class exercises seams for the FIRST time.
- [ ] #32 — real-network impairment: packet loss, latency/jitter, mid-body RST
      (proxy upgrade; eviction/retry seams at risk — lossy-but-alive must not be falsely evicted)
- [ ] #33 — server diversity: a third S3 implementation (`rclone serve s3`) + a second TUS
      lane if pinnable; every wire-fact delta pinned or honestly recorded (optional
      owner-cred lane for real AWS/B2 smoke)
- [ ] #34 — long horizon & resource honesty: overnight B/s leg, journal growth + RSS
      bounds, ENOSPC-while-journaling drill (what happens when TRUTH cannot be written)
- [ ] #35 — the real shell driven end-to-end: tauri-driver e2e on the ubuntu lane
      (create → progress → pause/resume → Done; cancel → no server object)

DoD: every probe closed with wire evidence; the readiness verdict in STATUS.md upgraded
per axis; any gap found files its own issue through the register discipline.
