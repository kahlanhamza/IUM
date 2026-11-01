# IUM MVP Specification v1.0

**Project:** Internet Upload Manager (IUM) · **Date:** 2026-11-01 · **Status:** adopted
**Evidence base:** `docs/01_Architecture_Teardown_v2.md` + dossiers in `rea-lab/out/`

---

## 1. Mission

A desktop upload manager that applies the proven architecture of download managers
(aria2, XDM, Motrix, FDM) to **uploads**: large files survive slow networks, laptop sleep,
crashes and reboots — and resume from the exact byte the server confirmed, not from zero.

Non-goals (v1): sync-style background mirroring, end-to-end-encrypted cloud albums,
browser-widget uploads, mobile apps (post-v1 via Tauri 2 mobile).

## 2. The stack decision

### 2.1 Chosen stack

| Layer | Choice | Why (evidence-tied) |
|---|---|---|
| **Transfer engine** | **Rust** (2021 edition, stable channel) | The engine is the product: per-chunk sockets, pacing, revocation, journaling — the exact class of work where aria2/FDM use C++. Rust delivers C++-class throughput with memory safety and a single-binary cross-platform story for community packagers. |
| **Async runtime** | **tokio** + **reqwest** (rustls) + hyper | Range-limited HTTP, `Expect: 100-continue`, per-request proxy, connection pooling; rustls avoids OpenSSL cross-compile pain on Win/macOS. |
| **App shell** | **Tauri 2** | Motrix proves Electron works but ships a full Chromium (~90–100 MB installers, ~150 MB idle RAM). Tauri 2 uses the system webview: ~10 MB installers, native tray/menus/notifications/single-instance/auto-updater built in, and a mobile path later. |
| **UI** | **TypeScript + React 18 + Vite + Tailwind + shadcn/ui** | Fastest community iteration; Zustand for state; Recharts for the throughput graph; react-i18next (FDM ships 85 locales — i18n is table stakes). |
| **IPC bridge** | **Tauri commands + events** (typed, semver'd) | IUM's version of Motrix's MBP1 lesson: a stable, versioned UI↔engine contract the future CLI/remote companion will reuse. |
| **Session store** | **SQLite via rusqlite, WAL mode** | Motrix (SQLite session logs) and FDM (bundled QSQLite + `AbstractDownloadPersistentInfo`) both converge here. Resume = replay of a durable journal, not guesswork. |
| **Protocol adapters** | `aws-sdk-s3`, `suppaftp`, `russh` + `russh-sftp`, custom thin clients (tus / Drive-resumable / Dropbox / Graph / WebDAV) | Each protocol is one module behind a trait (FDM's `AbstractDownloadModule` registry shape). |
| **Secrets** | OS keychain via `keyring` crate (Credential Manager / Keychain / Secret Service) | BYO OAuth creds (Drive/YouTube/Dropbox/OneDrive) never touch disk in plaintext. |
| **Integrity** | `crc32fast`, `md-5`, `sha2`, ETag parsing | Verify-before-declare (FDM `compareDownloadsByHttpEtag` teaching). |
| **Observability** | `tracing` + `tracing-appender` (rotating files) | Per-chunk spans like aria2's per-connection logs; user-attachable diagnostics. |
| **Testing** | `wiremock-rs` + custom **chaos proxy** (axum: throttle / drop / 429 / expire) | Prove the three signature behaviors: kill at 90%, slow-chunk eviction, tail racer. |
| **CI/CD** | GitHub Actions matrix → `tauri-action` releases (NSIS/MSI, DMG, AppImage/deb/rpm) | This repo now exists; signing can be added per-platform incrementally. |

### 2.2 Rejected alternatives (and the honest reason)

| Option | Verdict | Reason |
|---|---|---|
| **Electron + Node** (Motrix-style) | Rejected for core | Works (Motrix is proof) but: ~100 MB+ binaries, Chromium RAM floor, and Node's HTTP stack gives coarse control over per-socket pacing/revocation we need for chunk eviction. Our engine *is* the product; we want C++-class control. |
| **Go + Wails** | Considered, rejected | rclone proves Go is superb for cloud transfer, but the desktop-UI ecosystem is thinner, and Tauri 2's binary size + mobile roadmap fit the community project better. Revisit for a headless server variant — the engine crate is UI-agnostic by design. |
| **C++/Qt** (aria2/FDM path) | Rejected for a community MVP | Max control, slowest iteration, safety burden on volunteer contributors. |
| **Pure JS engine in the webview** | Rejected | Sandboxed filesystem access, no raw sockets, no true concurrency — the whole point of IUM is the opposite. |

### 2.3 Stack consequences (design rules)

1. **The engine crate (`ium-core`) must not depend on Tauri.** It is a library with a `JobManager`
   facade — this keeps the future CLI (`ium-cli`), a headless daemon, and tests on equal footing
   (Motrix's engine/UI split, done in-process).
2. **The UI never computes transfer truth.** Progress = replay of engine events; the SQLite
   journal is the single source of truth.
3. **Every protocol adapter compiles standalone** behind a feature flag; community contributors
   can add `ium-adapter-tus` without touching the S3 module.

## 3. System architecture

```
┌────────────────────────────── Tauri 2 shell (Rust) ──────────────────────────────┐
│                                                                                  │
│  React UI (webview)          typed IPC (commands+events, v1 API)                 │
│  ├─ Jobs list + badges  ◄───────────────────────────────┐                        │
│  ├─ Speed graph / per-chunk inspector                   │                        │
│  └─ Accounts (keychain-backed)                          │                        │
│                                                         ▼                        │
│  ┌──────────────────────────── ium-core (pure Rust lib) ───────────────────────┐ │
│  │  JobManager (supervisor state machine, pattern #5)                          │ │
│  │      │                                                                      │ │
│  │      ├── ChunkPool  ── reservation bitmap, initial split,                   │ │
│  │      │               dynamic resize, slow-chunk eviction (#1,2,9),          │ │
│  │      │               tail racer (#3)                                        │ │
│  │      ├── SessionJournal (SQLite WAL: jobs/chunks/checkpoints/events) #6     │ │
│  │      ├── TransferAdapter registry (#7 plugin shape)                         │ │
│  │      │     ├─ http_range   (PUT/Range, Expect:100-continue, ETag verify)    │ │
│  │      │     ├─ s3_multipart (aws-sdk-s3, ListParts reattach, checksums)      │ │
│  │      │     ├─ tus          (Upload-Offset/PATCH, creation-with-upload)      │ │
│  │      │     ├─ gdrive        (session URI, 308 Resume-Incomplete)            │ │
│  │      │     ├─ youtube      (resumable session URI)                          │ │
│  │      │     ├─ dropbox      (upload_session/append_v2 + commit)              │ │
│  │      │     ├─ onedrive     (Graph upload session URL, PUT chunks)           │ │
│  │      │     ├─ ftp          (REST/APPE + SIZE, RFC 3659)                     │ │
│  │      │     ├─ sftp         (russh-sftp append flags)                        │ │
│  │      │     └─ webdav       (PUT + If-Match/ETag verify; PATCH-append where  │ │
│  │      │                      supported)                                      │ │
│  │      ├── EndpointFailover (multi-origin per job, #8)                        │ │
│  │      └── RetryPolicy tables (per-error map, FDM teaching #9)                │ │
│  └──────────────────────────────────────────────────────────────────────────────┘ │
│   keyring (OS keychain)      tracing (rotating logs)      keychain-sealed config  │
└───────────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Job state machine (supervisor, pattern #5)

```
Created → Probing → Running ⇄ Paused
             │          │
             │          ├──→ Finishing (verify: ETag/checksum/ListParts)
             │          │         ├→ Done
             │          │         └→ Running (mismatch → re-verify/repair)
             │          └──→ Error(retryable?) → Running (backoff) | Failed
             └──→ Unresumable (protocol says one-shot and link died)
```

Every transition is **journaled before it executes**. Crash at any arrow = resume at the same
arrow after restart (journal replay).

## 4. Transfer core design

### 4.1 Chunk algebra

- `CHUNK_MIN = 5 MiB`, `CHUNK_MAX = 256 MiB` (S3 max part = 5 GiB, but 256 MiB keeps parts lists sane)
- Initial split: `n = clamp(maxConns, 2..32)`, `size0 = max(ceil(total/n), CHUNK_MIN)`
- Default `maxConns = 8` per job, global cap configurable (per-host accounting like aria2/FDM)
- Dynamic resize: every completed chunk reports throughput; the pool grows/shrinks the *next*
  chunk target between CHUNK_MIN and CHUNK_MAX (geometric moving average, α = 0.3)
- Progress persisted per chunk: `state ∈ {pending, leased, uploaded, verified}` + `bytes_confirmed`
  — **bytes_confirmed is only what the server acknowledged** (S3 ETag per part, tus offset,
  Drive committed range, Graph chunk response). Client-side optimism never enters the journal.

### 4.2 Slow-chunk eviction (pattern #2)

- Sliding window (10 s) throughput per worker; floor = `0.4 × median(pool)`
- Below floor for 2 consecutive windows → worker revoked, chunk returns to pool as `pending`
  with `bytes_confirmed` intact; a fast worker re-leases it and continues **from the confirmed
  offset**, not from zero.

### 4.3 Tail racer (pattern #3)

- When `remaining ≤ 2 × CHUNK_MIN`: duplicate unleased tail chunks across all idle workers
- Idempotency makes racing safe: S3 (same part number), tus (same offset PATCH), HTTP
  (PUT at same offset where server supports range-append) — first confirmation wins, losers' extra
  bytes are no-ops server-side or discarded by the adapter.
- Adapters that cannot dedupe (pure POST, some WebDAV PUTs) opt out and return
  `Capability::TailRace = false`.

### 4.4 Expired-session protocol (pattern #4)

| Signal | Meaning | Adapter action |
|---|---|---|
| HTTP 403/410 | presigned URL expired | re-sign (ask user/keychain), resume at offset |
| S3 `NoSuchUpload` | multipart session GC'd | ListParts probe → restart multipart at first unverified part |
| tus 460/409 | checksum/offset mismatch | HEAD → realign to server offset |
| Drive 308 w/ `Range` | resume-incomplete | continue at confirmed range |
| Drive 5xx / 429 | transient | `Retry-After`-aware backoff |
| Graph 416 | chunk already done | advance to next chunk |

### 4.5 Verify-before-declare (pattern #10)

S3: per-part `x-amz-checksum-crc32` + final ETag (multipart MD5 of part-MD5s when available).
tus: `Upload-Checksum` when negotiated. Others: client-computed SHA-256 logged in journal, and
(if the server offers) remote hash comparison. Mismatch → identify first bad chunk by binary
re-verify, re-upload that chunk only.

## 5. Protocol capability matrix (the honest badges)

| Protocol | Resume | Dynamic chunks | Tail race | Notes |
|---|---|---|---|---|
| S3 / R2 (multipart) | ✅ | ✅ (resizable parts) | ✅ (part-number idempotent) | ListParts reattach; checksums |
| Backblaze B2 (large file) | ✅ | ✅ | ✅ | `get_upload_part_url`; friendly-name failover |
| tus 1.0.0 | ✅ | ✅ (append-only) | ⚠️ (offset-append races are server-dependent) | `creation-with-upload` extension |
| Google Drive (resumable) | ✅ | ✅ | ⚠️ | 308 session URI; BYO OAuth |
| YouTube (resumable) | ✅ | ⚠️ (monotonic) | ❌ | session URI; BYO OAuth |
| Dropbox | ✅ | ✅ (append_v2 cursor) | ❌ (cursor is sequential) | commit at end |
| OneDrive / Graph | ✅ | ✅ (chunked PUT windows) | ⚠️ | 416 = chunk done |
| FTP (RFC 3659) | ✅ (REST/APPE + SIZE) | ✅ (parallel to distinct temp? no — single stream) | ❌ | single-stream resume |
| SFTP | ✅ (append) | ❌ | ❌ | single stream; offset via append flags |
| WebDAV | ⚠️ (PUT+verify; PATCH-append where supported) | ❌ | ❌ | probe `Accept-Ranges`-equivalent per server |
| Plain POST endpoint | ❌ **One-shot ⚠️ badge** | ❌ | ❌ | physically impossible; IUM still gives pause-retry + full-size temp staging |

UI rule: every destination shows its badge **before** the user commits a file to it.

## 6. Data model (SQLite, WAL)

```sql
CREATE TABLE jobs (
  id TEXT PRIMARY KEY,            -- ULID
  source_path TEXT NOT NULL,
  source_size INTEGER NOT NULL,
  source_sha256 TEXT,             -- computed incrementally
  adapter TEXT NOT NULL,          -- 's3_multipart' | 'tus' | ...
  endpoint TEXT NOT NULL,         -- normalized origin
  account_id TEXT,                -- keychain reference, never material
  state TEXT NOT NULL,            -- state machine value
  capability_badge TEXT NOT NULL, -- 'resumable' | 'one_shot'
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE chunks (
  job_id TEXT REFERENCES jobs(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL,
  offset_begin INTEGER NOT NULL, offset_end INTEGER NOT NULL,
  state TEXT NOT NULL,            -- pending|leased|uploaded|verified
  bytes_confirmed INTEGER NOT NULL DEFAULT 0,
  adapter_token TEXT,             -- part number / session cursor / etag fragment
  PRIMARY KEY (job_id, idx)
);
CREATE TABLE events (             -- journal replay + diagnostics export
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id TEXT, ts INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL
);
```

Retention: `events` rotated per job; a `PRAGMA wal_checkpoint(TRUNCATE)` runs on graceful quit.

## 7. Security model

- **Secrets:** OAuth refresh tokens / app passwords live in the OS keychain via `keyring`;
  the config file holds only keychain references.
- **Presigned URLs:** stored in memory + journal payload **redacted** (`…sig=REDACTED`).
- **Logs:** tracing spans carry job/chunk IDs, never headers/credentials; one-flag diagnostics
  export bundles sanitized logs + journal snapshot.
- **Updates:** Tauri updater with signed manifests (minisign keys held by release managers).

## 8. UI/UX plan (React)

- Jobs table: file, destination (logo + badge), progress ring, confirmed-vs-buffered bar,
  live speed, per-chunk inspector drawer (aria2-style transparency).
- Speed graph: 60 s rolling window; evicted-chunk markers.
- Accounts page: per-provider OAuth (BYO credentials), storage quotas shown.
- Badges: `Resumable ✅` / `One-shot ⚠️` / `Single-stream` chips with tooltips explaining *why*.
- i18n: react-i18next, en + ar + zh first (community can add locales — FDM's 85-locale bar).

## 9. Testing strategy

1. **Unit:** chunk algebra property tests (split/merge/eviction invariants).
2. **Mock server matrix:** wiremock-rs fixtures per protocol (S3, tus, Drive-308, Graph-416, FTP).
3. **Chaos proxy** (axum): scripted behaviors — drop at 90%, throttle to 10 KB/s (eviction),
   429 with `Retry-After`, expire presigned URLs mid-job, corrupt one part.
4. **Crash drills:** SIGKILL the app at each state-machine arrow; restart must resume from journal.
   This is the spec's acceptance gate, not a nice-to-have.
5. **Soak:** 1 GiB via chaos proxy on CI nightly (self-hosted runner if needed).

## 10. Platform & packaging

| Platform | Minimum | Bundle | Notes |
|---|---|---|---|
| Windows | 10 x64 | NSIS (Tauri) | tray, notifications, autostart |
| macOS | 11+ universal | DMG | Keychain, notarization when signing budget arrives |
| Linux | glibc 2.31+ | AppImage, deb, rpm | X11+Wayland (Tauri/WebKitGTK) |

## 11. Roadmap

| Sprint | Duration | Deliverable |
|---|---|---|
| **0 — Skeleton** | week 1 | repo+CI, Tauri 2 shell, `ium-core` crate with JobManager+SQLite journal, HTTP one-shot upload |
| **1 — Resumable core** | weeks 2–3 | ChunkPool + eviction + tail racer, HTTP-Range adapter, chaos tests, crash drills green |
| **2 — Cloud multipart** | weeks 4–5 | S3/R2/B2 multipart + ListParts reattach + checksums; tus adapter |
| **3 — OAuth ecosystems** | weeks 6–7 | Drive/YouTube/Dropbox/OneDrive, keyring, account pages, badge UX |
| **4 — Classic servers + polish** | week 8 | FTP/SFTP/WebDAV, diagnostics export, i18n en/ar/zh, v0.1 community beta |

### Sprint 1 task list (concrete)

1. `ium-core`: `ChunkPool` struct + property tests for split/merge invariants
2. Journal schema migration system (rusqlite + embedded migrations), WAL on
3. `TransferAdapter` trait + `Capability` flags + registry
4. `http_range` adapter: HEAD probe (`Accept-Ranges`, ETag) → PUT-at-offset loop
5. Eviction controller (10 s windows, 0.4×median floor) + unit tests with fake clocks
6. Tail racer (feature-gated by `Capability::TailRace`)
7. Chaos proxy crate with scripted scenarios 1–5
8. Crash drill harness (spawn core, SIGKILL at arrow N, replay, assert byte-level equivalence)
9. Tauri commands: `create_job`, `pause`, `resume`, `cancel`, `inspect_chunks`; events: `job_updated`
10. Jobs UI (list + progress + badge chips) on mock adapter
11. tracing setup + diagnostics export stub
12. CI: fmt + clippy + tests on all three OS targets

## 12. Risks & mitigations

| Risk | Mitigation |
|---|---|
| WebKitGTK quirks on Linux (media codecs, DPI) | Keep UI dependency-light; test on X11+Wayland early (Sprint 0) |
| S3 ListParts on GC'd sessions | `NoSuchUpload` path defined (§4.4); journal keeps part tokens so a fresh multipart only re-uploads unverified parts |
| OAuth app verification delays (Drive/YouTube quotas) | BYO credentials from day one (§5); IUM's own quotas are a v1.1 concern |
| WebDAV/POST servers without resume | Honesty badges + one-shot staging; no false promises |
| Scope creep toward sync products | Non-goals list in §1; every PR cites a spec section |

## 13. Clean-room statement

This specification was informed exclusively by: open-source source trees (aria2, XDM, Motrix),
and clean-room static observation of FDM 6.35 and IDM 6.43.16 (file layouts, symbol *names*,
string constants — dossiers in `rea-lab/out/`). No proprietary code, decompiled output, or
reconstructed logic was or will be copied into IUM.

---

*Next actions: Sprint 0 — scaffold `ium-core` + Tauri shell in this repo, first CI green, then
Sprint 1 acceptance = chaos-proxy crash drills passing.*
