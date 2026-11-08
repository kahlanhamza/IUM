#!/usr/bin/env python3
"""File issue #93: Sprint-2 UX — OS notifications for terminal job states."""
import json
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """## Design note (Sprint-2 UX lane 1 — the roadmap's remaining milestone work)

The Sprint-2 inventory (roadmap item 32a) is largely DONE already: drag-and-drop
sources landed with #65's zero-touch intake, settings persistence with #69/#86,
queue filters with the anatomy. Two real gaps remain: **OS notifications** (this
issue) and **the speed-cap control** (its own issue, core + shell).

### Wire facts

- `tauri-plugin-notification` v2: JS surface is `isPermissionGranted()` /
  `requestPermission()` / `sendNotification({ title, body })`; the capability
  grant is `notification:default` (adds to `capabilities/default.json` — the
  #40 lesson: nothing is granted by default).
- Linux renders through dbus/notify-rust. Under xvfb (the e2e lane) there may
  be no notification daemon — every send is guarded (`catch` → silent). The
  #80 silence lesson: an optional surface must never take the shell down.
- The shell already owns the truth about terminal states: `job_updated`
  snapshots + the 4 s poll both land in the same `jobs` array. One transition
  detector watches that array (ref map `id → state`), so listener, poll, and
  manual refresh take the SAME path — no double fires.
- Focus rule: `document.hasFocus()` at the transition — while the user is
  LOOKING at IUM, the toast system already speaks; the OS notification is for
  the away case. No new Tauri window APIs needed.

### The copy matrix (the whole design face — pinned by tests)

The OS renders the pixels; IUM owns the words. Voice = the copy table's
(sentence case, em-dashes, the engine's own words preserved on failure):

| state | title | body |
|---|---|---|
| done | `Upload complete` | `{basename} reached {adapter}.` |
| failed | `Upload failed` | `{basename}: {humanizeError(raw).title}.` |

- `basename` = the file name from `source_path` (never the full path — no
  machine layout leaks into the OS surface).
- `adapter` = the registry name (`webdav`, `s3`, ... — the same word the UI
  uses).
- `raw` = the last failure detail the journal carries; when the journal is
  quiet, the body is just `{basename} failed.` — never an invented cause
  (the #70 rule).

### Behavior matrix

- prev absent (first sight of a job) → never notify (app boot re-listing
  must not spam).
- `running|queued|paused → done` → one success notification.
- `any → failed` → one failure notification.
- same-state re-emit → nothing (idempotent).
- toggle off (Settings) → nothing; pref persists like the theme pref
  (localStorage, the useTheme store shape — a new tiny `notify.ts` store,
  no refactor of the theme module).
- window focused → nothing (the in-app toast already spoke).
- plain-browser dev (no Tauri) → the notifier seam is injected; the default
  no-ops, so vitest needs no Tauri runtime.

### Settings surface

One new card, exactly the approved card system's patterns (rows, toggles,
sentence): **Notifications** — "Tell me when an upload finishes or fails."
Nothing else; the feature is two terminal states, not a feed.

### Acceptance

- [ ] Pure transition module (`terminalNotify`): prev/next + options → draft
      or null; every matrix row pinned RED-first.
- [ ] `notify.ts` pref store (persisted, useSyncExternalStore), Settings card
      row wired to it.
- [ ] Plugin + `notification:default` capability; guarded sends; e2e lane
      unaffected (no pinned seam touched; xvfb silence proven by CI green).
- [ ] Copy matrix asserted in copy/voice gates; zero new design-system tokens.
"""

payload = {
    "title": "Sprint-2 UX: OS notifications for terminal job states (done / failed)",
    "body": body,
    "labels": [],
}
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps(payload).encode(),
    headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "ium-audit",
    },
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    d = json.loads(resp.read())
print(f"filed #{d['number']}: {d['title']}")
