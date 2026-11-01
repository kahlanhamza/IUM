# IUM Architecture Teardown v2 — What Download Managers Teach Us About Uploads

> **Provenance note:** v1 of this document (and its file-level line references) was lost to a
> sandbox recycle before it reached the repo. This v2 is reconstructed from the preserved session
> record and **re-grounded in fresh, reproducible evidence** gathered 2026-11-01
> (dossiers in `rea-lab/out/`). Claims are kept at the fidelity we can re-verify.

## 1. Targets studied

| Target | Type | Access | Key artifacts examined |
|---|---|---|---|
| **aria2** (C++) | open source | source tree | `SegmentMan.h` segment pool, slow-connection eviction |
| **XDM** (Kotlin/Java) | open source | source tree | `TailRacer.kt` endgame racing, 403/410 expired-link handling |
| **Motrix** (TypeScript/Electron) | open source | source tree | `engine-supervisor.ts` subprocess engine, SQLite session logging, MBP1 bridge protocol |
| **FDM 6.35** (C++/Qt6) | freeware | clean-room static (this study) | module layout + symbol names of `libdownloads{www,fragments,bt}.so` — see `FDM_6.35_engine_evidence.md` |
| **IDM 6.43.16** (C++/Win32) | commercial trial | clean-room static (this study) | installer container posture — see `IDM_6.43_static_evidence.md` |

## 2. The ten patterns → upload counterparts

Every serious download manager converges on the same ten ideas. Each has an upload twin, and
together they are IUM's transfer core.

| # | Download pattern (evidence) | Upload twin (IUM design) |
|---|---|---|
| 1 | **Segment pool** — one file = many ranged byte streams (aria2 `SegmentMan`; FDM `AbstractDownloadSection` + `IndependentSegmentsDownloader`) | **Chunk pool** — one file = many ranged uploads driven by a worker pool with a shared reservation bitmap |
| 2 | **Slow-connection eviction** — the pool reassigns ranges from lagging sockets (aria2; FDM per-host `QHash<QtHostAndPort,…>` accounting) | **Slow-chunk eviction** — chunks below a throughput floor are revoked and re-auctioned to fast workers |
| 3 | **Endgame racing** — duplicate the last ranges to idle connections, first writer wins (XDM `TailRacer`) | **Tail racer** — final chunks fan out to all free workers; server-side dedup is idempotent (PUT to same offset / same part number) |
| 4 | **Expired-link protocol** — treat 403/410 as "URL expired, re-resolve" not "fail" (XDM) | **Expired-session protocol** — S3 presigned expiry, Drive 308, tus 460/409 all trigger re-auth + reattach, never data loss |
| 5 | **Supervised engine process** — UI survives engine crashes (Motrix `engine-supervisor.ts`) | **In-process supervisor state machine** — engine lives in the Rust core; every state transition is journaled *before* it executes, so the process can die anytime |
| 6 | **Crash-safe session journal** — SQLite write-ahead log of task state (Motrix; FDM `AbstractDownloadPersistentInfo` + bundled SQLite driver) | **SQLite WAL journal** — jobs / chunks / checkpoints tables; resume = replay journal, not guesswork |
| 7 | **Versioned bridge protocol** — UI ↔ engine contract stays stable (Motrix MBP1) | **Typed Tauri IPC contract** — commands + events, semver'd, so the future CLI/remote companion speaks the same API |
| 8 | **Mirrors & failover** — same content from several origins (aria2 multi-source; FDM `DownloadMirrorsUsageManager` + `AbstractDownloadMirrorInfo`) | **Multi-endpoint failover** — same bucket via regional endpoints / direct+CDN host; B2 friendly names; alternate WebDAV hosts |
| 9 | **Adaptive scheduling & retry policy** — per-error retry decisions, speed-based rebalancing (FDM `DownloadErrorAllowedToAutoRetry`, `toDownloadScheduleRecord`; aria2 `--lowest-speed-limit`) | **Dynamic chunk resizing + policy tables** — chunk size grows/shrinks with observed throughput; 429/`Retry-After` respected (FDM ships the `Retry-After:` token) |
| 10 | **Integrity before "done"** — ETag/checksum validated against the resumed stream (FDM `WwwDownloadSection::compareDownloadsByHttpEtag`) | **Verify-before-declare** — ETag/`x-amz-checksum-*`/CRC32C/MD5 checked against the completed upload; multipart reattach via ListParts |

## 3. What the commercial study added (2026-11-01)

**FDM 6.35** (full dossier: `rea-lab/out/FDM_6.35_engine_evidence.md`):
- Confirms the **plugin-module registry** shape: one `AbstractDownloadModule` per protocol family,
  resolved by `AbstractDownloadsModulesProvider` → IUM's `TransferAdapter` trait registry mirrors this.
- Confirms **writer/network decoupling**: `IndependentSegmentsDownloader::setWritersThread(QThread*)` —
  network workers produce, one dedicated thread commits to disk. Upload twin: one writer drives the
  read-offset cursor; workers stream buffers to sockets.
- Confirms **ETag-validated resume** (`compareDownloadsByHttpEtag`) and `Expect: 100-continue`,
  `Retry-After`, HTTP/2 sniffing with fallback (`"Lying server, not serving HTTP/2"`), enterprise
  proxy support (PAC, NTLM).
- 85 locale files — i18n is not optional for community software.

**IDM 6.43.16** (full dossier: `rea-lab/out/IDM_6.43_static_evidence.md`):
- Ships a proprietary, Authenticode-signed installer container that defeats generic static tooling.
  We document the **posture**, and refuse to speculate on internals — IUM's architecture claims rest
  on the four observable targets instead.
- Lesson for distribution: single-file bootstrap, code signing, per-browser integration components
  are the polish bar for a "professional" manager.

## 4. Consequence for IUM

The download world already proved the transfer architecture. IUM's novelty is the **mapping**:
every pattern above has a protocol-specific upload twin, and the product's honesty lives in its
badges — upload resume is a *protocol negotiation*, not a checkbox.
The stack and module design that implement this mapping are specified in
`02_IUM_MVP_Spec_v1.md`.
