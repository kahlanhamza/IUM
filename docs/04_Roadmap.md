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
- [ ] S3 multipart adapter, tested against MinIO in CI — one pinned binary per OS runner
      (services containers are Linux-only; spike doc docs/07 §6) — issues #6–#9
- [ ] TUS 1.0.0 adapter against a real test server
- [ ] Resume across process restart for both (kill -9 drill in CI on Linux)
- DoD: end-to-end uploads of >100 MiB with forced mid-body kills; zero corruption on resume;
      honest Done only after adapter-verified completion.

## M4 — App shell (Tauri 2 + React) — **FROZEN**
Unlocks only after M2 + M3. pnpm→npm already fixed in `tauri.conf.json`.

## M5 — Public API freeze & docs
- [ ] `ium-core` public API reviewed and documented (`cargo doc` clean)
- [ ] Versioning policy (semver), changelog discipline
