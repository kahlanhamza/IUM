# GREEN evidence — #91 (the ⌘K command palette)

Commit `97e2260` — the approved mockup (`design/mockup/palette-*.html`) became
behavior. Owner approval on the record ("Ok go ahead", 2026-11-08).

## RED → GREEN

RED at HEAD: `src/command-palette.test.tsx` — **14 failed / 1 passed** (module
unresolvable + every behavior pin failing; the one pass is the trivial
"closed until summoned"). GREEN on the lane commit: **144/144 across 17 files**
(was 129/129 across 16 — +15 palette pins, zero regressions), tsc strict,
build 212.75 KB JS / 66.09 KB gzip, axe clean with the dialog open,
CI **8/8** (run 42493687154) incl. app-e2e on WebKitGTK — the palette
disturbed no pinned seam.

## What shipped

- `components/CommandPalette.tsx` — 640 px surface at 96 px over its own smoke
  scrim (the Settings modal's width token), 36 px rows, ink active row, mono
  keycaps, hairline footer; filter + arrow-wrap + Enter + esc + row keycaps
  while open; focus restore to the summoner; `aria-activedescendant`; the
  honest empty state. Zero new deps; the only animation rides `--dur` (150 ms).
- The honest seven (the mockup's two API-less rows never shipped — "Retry
  failed uploads" → no engine retry transition; "Remove completed" → no API):
  New upload… (⌘N) · Pause all active uploads · Resume all paused uploads ·
  Check for updates · All uploads (⌘1) · Connect a destination… (⌘3) ·
  Open Settings (⌘,).
- Batch verbs answer zero candidates honestly ("No active uploads to pause.");
  the user-initiated update check speaks both ways (the boot check stays silent).
- Global reflexes: ⌘K/Ctrl+K toggle, ⌘N, ⌘, — platform-true glyphs
  (⌘ on macOS, Ctrl elsewhere) in the rows and the new toolbar **Commands** chip
  (accessible name contains its visible text — the axe label-mismatch rule).
- Visual check: the real app in a browser (vite + headless) — chip, scrim,
  groups, keycaps and footer render exactly per the mockup (screenshot on the
  record in the lane report).

## Visual verification

Rendered through the real stylesheet in the running app (plain-browser dev
mode; the engine-unreachable toasts are the no-Tauri degradation, not a
defect): the palette shows ACTIONS (4 rows, ink active on "New upload…") and
GO TO (3 rows) with ⌘-per-platform keycaps and the ↑↓ navigate · ↵ run ·
esc close footer.
