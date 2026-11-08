#!/usr/bin/env python3
"""File issue #91: the ⌘K command palette — keyboard-first (P2)."""
import json
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """## Design note (the approved mockup → honest commands)

Owner-approved 2026-11-08: `design/mockup/palette-light.html` + `palette-dark.html` —
the ⌘K palette over the Ink anatomy. Surface language: 640 px panel (the Settings
modal's width token) anchored 96 px from the window top, smoke scrim (`--scrim`),
radius `--r-card`, the floating-surface shadow law, 36 px rows (the content-row
rhythm), ink active row (wash + 500), JetBrains Mono keycaps, hairline footer
(↑↓ navigate · ↵ run · esc close), fade/ rise ≤ 150 ms (`--dur`).

The toolbar gains the mockup's entry chip on the right: **Commands ⌘K** (a real
button; the keycap glyph follows the platform: ⌘ on macOS, Ctrl elsewhere).

### Command table — the mockup made honest

The mockup listed 7 rows; two had no engine API. The palette ships only commands
that are real today (the UI never lies about what the engine says):

| # | Group | Command | Keycap | Wiring (exists today) |
|---|---|---|---|---|
| 1 | Actions | New upload… | ⌘N | `focusNewUpload()` — the #upload-path seam |
| 2 | Actions | Pause all active uploads | — | `pause_job` per active job (probing/running/finishing) |
| 3 | Actions | Resume all paused uploads | — | `resume_signal` per paused job |
| 4 | Actions | Check for updates | — | `checkForUpdate()` (#80), result toasts |
| 5 | Go to | All uploads | ⌘1 | `setFilter("all")` |
| 6 | Go to | Connect a destination… | ⌘3 | opens Settings (the sidebar's own `sadd` path) |
| 7 | Go to | Open Settings | ⌘, | opens Settings |

Deviations from the mockup, on the record:
- *"Retry failed uploads" → replaced by "Resume all paused uploads"*: the engine
  has no retry transition (failed jobs have no UI path today); resume exists.
  A real retry is an engine lane of its own, filed separately when ordered.
- *"Remove completed from list" → dropped*: no remove API. Keeping the row would
  render a command that cannot run.

### Keyboard map

- **Global (window-focused):** `⌘K`/`Ctrl+K` toggles the palette; `⌘N` new
  upload; `⌘,` opens Settings. Standard desktop reflexes, both platforms.
- **Palette open:** `↑`/`↓` move the active row (wrapping), `↵` runs it,
  `esc` closes, typing filters (case-insensitive substring on labels), and the
  row keycaps fire their commands while open (`⌘1`, `⌘3`, `⌘,`).
- Not in scope: system-wide (window-unfocused) shortcuts — that would add the
  global-shortcut plugin (zero-new-deps law) and desktop convention keeps
  in-app hotkeys in-app.

### A11y + gates

- `role="dialog"` + `aria-modal` + label; list = `listbox`, rows = `option` +
  `aria-selected`; focus lands in the input on open and returns to the
  previously focused element on close; axe clean with the palette open.
- Batch actions with zero candidates answer honestly ("No active uploads").
- The #65-pinned ids and the #88 anatomy pins are untouched; the palette is
  pure addition (a new component + the toolbar chip + CSS).
- RED first: `command-palette.test.tsx` fails at HEAD (module missing).

## Acceptance

- [ ] RED at HEAD, then GREEN on the lane commit.
- [ ] All 7 commands wired and tested; batch honesty (zero-candidate sentence).
- [ ] ⌘K/Ctrl+K both platforms; ⌘N/⌘, global; arrow-wrap; Enter runs; Esc closes.
- [ ] axe clean (dialog open state included in a11y.test.tsx or the lane's own).
- [ ] Gates: vitest + tsc + build + core suite; CI 8/8.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps(
        {
            "title": "The ⌘K command palette — keyboard-first (P2, the approved mockup)",
            "body": body,
            "labels": ["app", "design"],
        }
    ).encode(),
    headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
    },
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    out = json.load(resp)
print(f"filed issue #{out['number']} url={out['html_url']}")
