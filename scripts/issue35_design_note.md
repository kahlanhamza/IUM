# #35 design note — real-shell WebDriver e2e (the shell ↔ supervisor seam, driven once)

## Architecture (mirrors the wire-suite harness idiom: harness shell script + env-gated test)

```
scripts/app-e2e.sh (orchestrates, CI = the proof lane)
  1. ium-app: npm ci + tsc strict + vite build          → dist/
  2. cargo build -p ium-app (the Tauri shell binary)     → target/debug/ium-app
     cargo build -p loopback-recorder (new bin, ium-core)→ target/debug/loopback-recorder
  3. loopback-recorder --port P --throttle-bps 2097152
     --record-file /tmp/e2e/recorder.json &              (the "HTTP loopback recorder")
  4. xvfb-run tauri-driver --port 4444 &                 (WebKitWebDriver underneath)
  5. cargo test -p ium-e2e --test e2e_shell              (env-gated: IUM_E2E_*)
```

- **Driver**: `tauri-driver` (cargo install --locked, rust-cache-cached). Linux only
  (WebKitWebDriver, apt `webkit2gtk-driver` — verified: ships `/usr/bin/WebKitWebDriver`
  on noble). xvfb-run provides the display WebKitGTK needs; WebKitWebDriver spawns the
  app binary itself, so DISPLAY flows tauri-driver → WebKitWebDriver → app.
- **Session contract** (read from tauri-driver source, not guessed): `POST /session`
  `capabilities.alwaysMatch["tauri:options"] = { application: "<app bin path>" }`;
  tauri-driver translates to `webkitgtk:browserOptions {binary, args}`.
- **Client**: hand-rolled minimal WebDriver client (plain W3C JSON-over-HTTP on
  reqwest — session, find css selector, send keys, click, text, attribute) inside the
  test. No new dependency: the engine hand-rolls sigv4 for the same reason. WebSocket
  endpoints (bidi) are NOT needed for element driving.
- **New workspace member `ium-e2e`**: deps tokio + reqwest(json) + serde_json only —
  does NOT link tauri/ium-app, so it compiles in any sandbox (webkitgtk not needed to
  compile). Without `IUM_E2E_DRIVER` set, every test skips LOUDLY (the same env-gated
  idiom as the S3/TUS wire suites) — the offline lane stays green everywhere.
- **The recorder** (`ium-core/src/bin/loopback_recorder.rs`): the http_one_shot
  TestServer semantics as a process — PUT records the body + 200/ETag, HEAD reports
  held bytes, **404 when nothing is held** (an object store's honest "no object"),
  optional `--throttle-bps` (paced body reads — the deterministic-clock lesson from
  #28/#32: control the wire, don't race it), `--record-file` JSON `{put_count,
  received_bytes}` rewritten after every request. Reuses none of the test file's code
  (no drive-by refactor of http_one_shot.rs — the test copy stays as the wire suite's
  pin).

## The flow driven (acceptance criteria → test steps)

1. app boots → status line reads `ium-core vX ready — adapters: http_oneshot, tus`
   (the UI reflects the registry — also pins #16's list_adapters through the real IPC).
2. create: file path entered (see the seam below) → endpoint
   `http://127.0.0.1:P/upload` → adapter `http_oneshot` → connections 3 → Upload.
3. live progress: percent text in the first job row increases across `job_updated`
   ticks (400 ms cadence — asserted: two reads, second > first, while Running).
4. pause → state chip `paused` (cooperative flag lands between chunk leases; the
   12 MiB file at ~2 MiB/s through the throttled recorder gives a multi-second
   Running window) → resume → chip returns to `running` → `done`.
5. Done badge + inspector: row click opens the drawer (inspect_chunks through REAL
   IPC) — assert chunk rows confirm the full size, states uploaded/verified, journal
   events listed; recorder JSON shows received_bytes == file size, put_count ≥ 1.
6. cancel leg (second job, bigger file / same throttle): Running observed → Cancel →
   chip `canceled` → recorder JSON `received_bytes == 0` and a direct HEAD → 404 —
   a canceled upload never completed the object (the #15/#26 invariant through the UI).

## The seam the e2e forces (registered, minimal, product-honest)

The create form's only path entry is the native file-picker button — GTK dialog,
undrivable by WebDriver (and unscriptable in CI). Minimal seam: the path display span
in CreateJobForm becomes a **text input bound to the same state** (paste-a-path is a
real power-user feature; the picker still fills it). The e2e fills it. Same commit
adds WebDriver-stable handles to the EXISTING controls only: `id`s on the four form
controls, `aria-label`s on the row action buttons (accessibility-positive),
`data-state` on the state chip. No engine code changes; shell Rust untouched.

## RED plan

Commit 1 ships the test + harness WITHOUT the path seam: on CI the drive fails at
"enter the source path" (element-not-found on `#upload-path`) — the failing output
goes on this issue. Commit 2 adds the seam → GREEN. Local RED support: the e2e crate
compiles + loud-skips in the sandbox (no WebKitGTK — the shell Rust lane cannot build
here; the frontend lane (tsc/vite/vitest) proves the seam commit locally).

## Out of scope (per issue)

Visual snapshots; Sprint-2 features; macOS/windows e2e (no native WD for WKWebView on
macOS — the ubuntu lane is the issue's scope).
