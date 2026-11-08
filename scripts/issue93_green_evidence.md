GREEN evidence — #93 (terminal notifications)

Commit `6a96623` — **CI run 188 (42516234143): 8/8 success on the FIRST
verdict** (the #92 lesson applied: the Rust-side API was verified against
the real crate source — `tauri-plugin-notification` 2.8.0,
`NotificationExt::notification()` → `builder().title().body().show() ->
Result<()>` — before the push, so the CI-only shell lane compiled clean on
all three OS first try).

## RED → GREEN

- RED at HEAD `059d76e` (comment 6726052177): the 11 behavior pins cannot
  resolve `./notify` — `Failed to resolve import "./notify"`; no
  notification code existed anywhere in the shell.
- GREEN: **157/157 across 18 files** (was 146/146 — +11 notification pins),
  tsc strict, gate.sh GREEN on the committed tree, brand gate extended to
  walk the static notification strings (voice rules keep their teeth), CI
  8/8 incl. app-e2e on WebKitGTK — no pinned seam disturbed.

## What shipped

- `notify.ts` — the pure transition rule (`notifyDraftFor`): first sight
  silent (boot re-listing never spams), same-state idempotent,
  `running|queued|paused → done` and `any → failed` notify once, focused
  window silent (the toast already spoke), toggle off silent; the persisted
  pref rides the theme store's shape (localStorage, default on) + the
  basename rule (POSIX and Windows separators).
- ONE detector in App.tsx watches the jobs array — the live `job_updated`
  listener, the 4 s poll, and manual refresh all land in the same array, so
  no path double-fires; the failure's raw words come from the newest event
  payload carrying `error` (the copy table humanizes; a quiet journal means
  the honest no-cause body).
- The copy matrix (`NOTIFICATIONS_COPY`, brand-gated): "Upload complete" /
  "Upload failed" + the Settings sentences; bodies carry the file basename
  only — no machine paths leave the machine.
- Settings → **Notifications** card: one switch + the focus-rule sentence,
  exactly the approved card system.
- Rust: `notify_user` command (`app.notification().builder()...`) registered
  in `generate_handler`; `tauri-plugin-notification` crate only — ZERO new
  JS packages; no capability grant needed (the ACL gates webview IPC, not
  in-process calls); every send guarded — a missing daemon (headless e2e)
  or denied permission is a silent no (the #80 lesson).

Closing completed.
