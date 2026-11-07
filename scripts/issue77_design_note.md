# The design pass: quiet chrome, a real logomark, a README with receipts

**Lane:** #66 (the product bar — design system, UX, brand). **Kind:** product-design
corrective pass, driven by the owner's verdict on the first screenshot set: *"not even
near big tech companies perfection … avoid AI slop designs … the header label … this is
not like big tech teams are doing it … the software icon must be professional and
creative … then take screenshots again and put them in the readme … the readme is
outdated."*

## 1. The principles this pass applies (the research, named)

The owner's instruction is that design research is the engineer's job here. These are
the rules this lane applies, with their sources of authority:

1. **The chrome is quiet; the content is loud.** Apple HIG ("defer to content"),
   VS Code's titlebar, Chrome's toolbar: the persistent header carries identity and at
   most one ambient signal. Version strings, adapter rosters and internal component
   names are **About-dialog material, never header material**. IUM's header currently
   renders the engine's raw status sentence — `ium-core v0.1.1 ready — adapters:
   http_one_shot, s3_multipart, tus` — as a permanent pill. That is a debug readout
   wearing a UI hat. (D1 below.)
2. **No decoration without information.** Big-team visual systems (Fluent 2, Material 3,
   Linear, 1Password) spend ink only where it encodes state or hierarchy: weight, size
   and one accent color do the work; letter-spaced uppercase section headers, gradient
   logo tiles and animated sheen fills are decoration claiming hierarchy they do not
   carry — the recognizable "template" tells. (D2, D4.)
3. **A brand mark is a constructed object, not an emoji of intent.** Apple/Fluent icon
   practice: geometric construction on a grid, optical balance, flat two-tone, legible
   from 16 px favicon to 512 px installer, one source of truth (SVG paths) feeding every
   surface. IUM currently has **no mark at all** — the header reuses the generic
   `IconUpload` stroke icon on a gradient tile. (D3.)
4. **Typography is the interface.** One family, a small ramp, sentence case, hierarchy
   by weight and color — not by shouting. (D4.)
5. **States are designed, not defaulted** — hover/active/focus-visible/disabled on every
   control; status rendered honestly but quietly (dot + two words; the engine's own
   words remain inspectable in tooltip and About). (D1, D2.)
6. **The brand voice survives the polish.** Honest badges, raw engine words, the
   journal — nothing is hidden; the redesign reframes where truth *surfaces*.

## 2. Wire facts (measured at HEAD `3857b93`)

- `App.tsx` renders `engine.detail` verbatim inside `.engine-pill` — the string is
  `format!("ium-core v{} ready — adapters: {}", VERSION, names)` (`src-tauri/src/lib.rs:212`).
- No About surface exists anywhere; the version has no home.
- `theme.css` is a real token system (light+dark, #73-gated) — the foundations are
  good; the slop is in specific rules: gradient `.brand-mark`, uppercase + 0.8 px
  letter-spacing on `.section-title`/`.table th`, animated sheen on `tr[data-active]
  .progress-fill`.
- No favicon is declared (`index.html` has no `<link rel="icon">`); `src-tauri/icons/`
  carries the Tauri default set; no master SVG exists in-repo.
- README carries zero screenshots and a stale roadmap section ("M0–M7 … toward v0.1.0";
  the tree is at v0.1.1 with product-bar lanes 1–8 closed, #74/#75/#76 landed).

## 3. Decisions

- **D1 — Quiet status.** The pill shows dot + two words: `Connected` /
  `Connecting…` / `Engine unreachable`. The engine's full sentence moves to the pill's
  `title` tooltip and, permanently, into a new **Settings → About** footer (name,
  version from `package.json` via a `__APP_VERSION__` define, tagline, the engine's own
  detail line). Header shows no version, no adapter list, ever.
- **D2 — De-slop the rules.** Section headers and table headers: sentence case, weight
  + color hierarchy, no letter-spacing games. Progress fill: solid accent; the sheen
  gradient is deleted. The active row's liveness is already encoded by the state chip
  and the speed column — motion is not duplicated on the fill.
- **D3 — The IUM logomark.** Constructed SVG paths, two flat tones (teal `#0F766E`
  tile — the brand accent's light-scheme value — and white glyph): a rounded-square
  tile carrying a bold "U" whose counter holds an upward arrow — *upload* inside the
  *U* of IUM. One master (`design/logo.svg`, 512 viewBox) feeds: the in-app header
  `Logomark` component, `public/favicon.svg`, and the full Tauri set via
  `npx tauri icon` (PNG/ICO/ICNS regenerated in-tree). Documented in the brand sheet
  (new "Logomark" section — additive, the #73 gate checks a subset).
- **D4 — Type ramp cleanup.** 13 px section titles (650), 11.5 px table headers (600,
  sentence case, `--text-3`), body unchanged; odd 13.5 px values normalized.
- **D5 — README with receipts.** New top "Screenshots" section embedding
  `docs/screenshots/01…06` (the committed product shots, dark first — the default
  scheme). Status & roadmap rewritten to the real state: M0–M9, product-bar lanes 1–8,
  telemetry stance (#75), RE-resistance bar (#76); next steps = #59/#60 (owner-parked),
  #65 release bar (signing cert pinned by owner). Badge counts refreshed to measured
  numbers. A `readme.test.ts` gate pins: all six screenshots referenced, no stale
  "v0.1.0" roadmap, the word "Privacy"/telemetry stance present.
- **D6 — The e2e seams are untouchable.** `#upload-*`, `#app-settings`, option VALUES,
  semantic table structure, `[data-state]` — presentation only. The a11y gate re-audits
  the composed shell (it renders App with the api mock; the pill's new copy rides the
  same render path).

## 4. RED first (vitest)

1. `header.test.tsx` — renders the full `App` with a mocked `api.engineStatus`
   resolving the real engine sentence; asserts the header never renders a version
   pattern (`/v\d+\.\d+\.\d+/`) or the adapter roster, renders `Connected`, and the
   logomark SVG exists. Fails at HEAD (detail is rendered verbatim; no Logomark).
2. `readme.test.ts` — reads `README.md`; asserts the six screenshot embeds exist and
   the stale v0.1.0 roadmap line is gone. Fails at HEAD.

## 5. Acceptance

- vitest suite green (new gates included), `tsc && vite build` green, cargo trio green
  (untouched paths, re-run anyway — trust nothing not executed).
- New screenshots captured from the running app (vite dev + the Tauri IPC shim in the
  screenshot harness — the lost recycle artifact rebuilt, committed), committed to
  `docs/screenshots/`, embedded in README.
- One logical commit, STATUS.md evidence row, push, CI green on 3 OS, issue closes on
  the commit.
