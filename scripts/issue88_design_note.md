### Design note — the anatomy pass, from the wire facts up

**Wire facts (read from the tree, not guessed):**

- W1 — the e2e seams that are LOAD-BEARING (`ium-e2e/tests/e2e_shell.rs`,
  pinned #65): `#upload-path/-endpoint/-adapter/-connections/-submit`;
  rows via `table tbody tr:nth-child(i)`; file name in the FIRST td; the
  FIFTH td's text starts with `NN%`; `[data-state]` chip per row;
  `button[aria-label^='pause'/'resume']` in-row; the inspector found by
  `[role="dialog"]`, whose FIRST button is Close, and whose chunk rows are
  read as `table tbody tr td:nth-child(3)` (state) / `td:nth-child(4)`
  (confirmed). `#app-settings` appears in the header comment only — the
  lane never drives Settings.
- W2 — `JobsTable` is a semantic `<table>` and its own header comment
  declares the structure load-bearing. The Ink mockups draw div rows, but
  nothing in the approved design requires DIVs — the mockup's LOOK
  (36 px, hairline, edge-to-edge, tabular figures) is a styling of rows,
  and the semantic table IS the better accessibility tree. **The table
  stays; the mockup's look arrives as CSS.** The 5th-td progress column
  keeps leading with `NN%`.
- W3 — the one place the mockup and the seam genuinely collide: the
  inspector. The mockup's middle column is a chunk GRID (16-wide squares),
  the e2e reads a chunk TABLE. Per D4 this seam move is legal IN THE SAME
  COMMIT: the docked pane renders the chunk grid, every square carrying
  `data-idx`, the raw journal state in `data-state`, and `data-confirmed`
  (bytes) — journal truth rendered verbatim as data attributes, Σ still
  assertable. The e2e leg is amended to read the grid in this commit.
- W4 — `credentialsSchema()` + `credentialsGet()` already expose every
  lane's fields with `present` flags: the sidebar's Destinations dots are
  the vault's own truth (all-required-present = connected), one IPC call.
- W5 — the engine is IN-PROCESS in the packaged app: no address exists.
  The status bar shows the honest short forms — engine word + parsed
  version (`v0.1.1`) — never the raw sentence (#77's law keeps holding;
  the header test's `not /ium-core v/` stays true shell-wide).

**The design:**

- D1 — **Anatomy** (the mockup's metrics exactly): `header.toolbar` 48 px
  chrome + hairline — New Upload (primary) | sep | spacer | ThemeToggle |
  sep | `#app-settings`. **NO brand block** (the ladder's Chrome rung:
  none — enforced in the component by a new pin, not only the sheet).
  `nav.sidebar` 210 px: Library (All uploads / Active / Queued /
  Completed / Failed — real filters with counts, `aria-pressed`) +
  Destinations (the vault lanes with connection dots) + Connect
  destination… (opens Settings). `.content`: the create panel (the form
  stays the front door — an upload-first product; restyled to an inset
  panel, hairline-carried, every `#upload-*` seam untouched), then the
  jobs table full-bleed (thead styled AS the mockup's `.chead`: 30 px,
  micro-caps), rows 36 px hairline-separated, tabular figures on the
  numeric columns. `footer.statusbar` 28 px: live counts (mono) left;
  engine dot + word + short version right. The docked inspector
  (`[role="dialog"]` kept, Close first, Esc kept) — 150 px, border-top,
  panel bg, three columns (summary / chunk grid / journal), PUSHES the
  window (a flex child, never `position: fixed`). Settings: `.scrim` +
  `.modal` 640 px centered, shadow ONLY here, Esc ladder untouched.
- D2 — **Retired**: `.app-header` masthead, `.brand` block, `.app-main`
  max-width centering, `.section-head`/`.count-chip` (counts live in the
  sidebar and status bar now), `.card` floating cards, the inspector
  overlay + backdrop, the settings drawer chrome. `.drawer-backdrop` and
  `.drawer` leave theme.css (the modal's internal typography classes are
  kept as names, re-scoped).
- D3 — **The shadow law, enforced**: `box-shadow` in theme.css appears
  only under the floating surfaces (.modal, .scrim tokens, .toast) — the
  checked-theme-option pill loses its shadow to a hairline. No
  backdrop-filter anywhere (pin).
- D4 — **Lockstep**: `header.test.tsx` rewritten (toolbar carries no
  brand; quiet-status law re-homed to the status bar); new
  `anatomy.test.tsx` pins the metrics from the REAL stylesheet via
  node:fs (48/210/36/28/640, docked-not-fixed, shadow law, no
  backdrop-filter, no `.drawer`); filter behavior pinned; New Upload
  focuses the path field; `logic.parseVersion` + pin; the e2e chunk leg
  + header doc amended in this commit; axe re-run over the new shell
  (sidebar nav labelled, sitem buttons named, docked dialog unchanged).
- D5 — **Performance**: zero new JS dependencies; the only new motion is
  the existing ≤150 ms opacity vocabulary; no backdrop-filter.

**Deferred, on the record**: the mockup toolbar's batch Start/Pause/
Remove (no batch selection exists — a behavior change, not a restyle) and
the ⌘K search field (the keyboard-first pass, P2) are NOT in this lane;
this note puts them on the record as P2 candidates.

RED follows as a comment (the new pins failing at HEAD), then the swap.
