# The IUM brand sheet

**Product bar #66, lane 6 (#73); the Ink pass (#86).** One page; the decided
identity of the product. Every value here is lifted from the tree —
`theme.css`, `copy.ts`, `logic.ts`, `App.tsx` — nothing aspirational.
`ium-app/src/brand.test.ts` enforces it: the sheet cannot drift from the
tokens it documents, and the UI copy must follow the voice it declares.

## What IUM is

An upload manager that applies download-manager reliability engineering to
uploads: resumable, crash-safe, verified before it says Done. The brand
consequence is the whole voice: **the product speaks the engine's truth.**
It reports what happened, never what sounds nice.

## Name and wordmark

- The name is **IUM** — three letters, always uppercase, never "Ium".
- The expansion is **Internet Upload Manager** — the README title's form and
  the header's sub-line (`App.tsx`); no other tagline exists or is invented.
- The lockup is the **Logomark** (see below) beside **IUM**, the expansion
  as the sub-line. Version numbers and engine details belong to Settings →
  About, never to the lockup (#77's quiet-chrome rule).
- The engine crate is `ium-core` — lowercase, code context only. The desktop
  app is IUM; "the IUM app" is not a name.

## Voice

Recorded from the copy table (`copy.ts`, #70) — the rules the sentences
already obey:

1. **Honest over reassuring.** The raw engine words stay visible: "the
   engine's own words are below — they help report the problem."
2. **Specific over generic.** Name what failed and where: "The destination
   rejected the credentials", not "Something happened".
3. **Action over apology.** Every failure carries a hint — or says why none
   is needed: "The engine waits and retries automatically — no action needed."
4. **Calm.** No exclamation marks, no emoji, no blame: "the protocol's
   limit, not a bug."
5. **The engine does the work.** "backs off automatically", "re-sends the
   affected parts", "resumes from where it stopped" — the user decides, the
   engine labors.
6. **The journal is the truth.** "the list shows the journal's truth" — the
   UI reports state; it never invents it.

## Naming rules

Product surfaces are friendly; engine seams stay technical.

| Engine seam (code, docs) | UI label (`ADAPTER_LABELS`) |
| --- | --- |
| `http_oneshot` | HTTP one-shot |
| `tus` | TUS 1.0.0 |
| `s3_multipart` | S3 multipart |
| `feishu_drive` | Feishu Drive |
| `terabox` | TeraBox |
| `mega` | MEGA |

- Unknown adapter ids render raw (`friendlyAdapter` falls through) — the UI
  reflects the engine; it never invents lanes.
- `adapter_terabox`, `JobsTable`, `ChunkPool` and friends stay verbatim in
  code and engineering docs; the friendly label is a UI surface only.

## Color

**Two inks, two schemes (#86, owner ruling).** The brand color is the
logomark's ink — `#0F172A`, the near-black of the dart — and its inverted
twin, the near-white ink the same mark wears on dark surfaces. Light mode
speaks the dark ink on paper white; dark mode speaks the light ink on the
midnight chrome. There is no third hue: **the accent IS the ink** — filled
controls are ink with the scheme's opposite type on them.

Color is reserved for meaning: red speaks ONLY failure, green speaks engine
health, and nothing else gets to shout. Terminal success (Done) is the
default expectation and stays neutral ink; paused is a user action, not a
caution. Semantic colors always travel as a foreground/background pair.
The table is the projection of the tokens; `brand.test.ts` requires exact
equality with `theme.css` in both schemes (light on `:root`, dark on
`[data-theme="dark"]` — light is the no-attribute base, so a pre-paint
frame can never flash the wrong scheme).

| token | light | dark |
| --- | --- | --- |
| `--bg` | `#f5f6f8` | `#0b101a` |
| `--surface` | `#ffffff` | `#0e1521` |
| `--surface-2` | `#eef0f4` | `#131c2c` |
| `--surface-3` | `#e4e8ee` | `#18233a` |
| `--text` | `#0f172a` | `#edf1f8` |
| `--text-2` | `#475569` | `#a8b3c7` |
| `--text-3` | `#5e6d86` | `#77849d` |
| `--border` | `#e6e9ef` | `#1d2942` |
| `--border-strong` | `#d2d8e3` | `#2c3a5c` |
| `--accent` | `#0f172a` | `#e9eef6` |
| `--accent-hover` | `#1d2941` | `#f6f9ff` |
| `--accent-down` | `#0a1122` | `#d5deeb` |
| `--accent-text` | `#ffffff` | `#0b101a` |
| `--accent-subtle` | `#eef1f6` | `#1a2438` |
| `--ok` | `#2b7549` | `#7cc793` |
| `--ok-bg` | `#e7f3ec` | `#16301f` |
| `--danger` | `#c43a2b` | `#ff8f86` |
| `--danger-bg` | `#faeceb` | `#331d22` |
| `--warn` | `#8f610c` | `#e3b95f` |
| `--warn-bg` | `#faf0dc` | `#33290f` |
| `--info` | `#40639f` | `#92aae6` |
| `--info-bg` | `#eaeffb` | `#1a2540` |

The focus halo (`--accent-ring`) is a rgba value on purpose — it is motion
state, not a brand color, and lives in `theme.css` only. Geometry (10 px
cards, 6 px controls, pill radius) and motion (150 ms, reduced-motion
respect) are theme.css territory; the sheet records the fact, not the
numbers.

## Type

**The brand ships its own type (#86, owner ruling).** Inter speaks the UI
(weights 400/500/600), JetBrains Mono speaks the machine — endpoints,
adapter ids, byte counts, chunk grids, the "the engine said: …" raw line.
Both ride as vendored latin-subset woff2 files (`ium-app/public/fonts/`,
~140 KB total, `font-display: swap`, provenance @fontsource 5.3.0, SIL OFL
1.1 — the licenses ship beside the files).

- **Why vendored:** the cross-OS guarantee. macOS, Windows and Linux all
  render the SAME faces instead of drifting through SF/Segoe/Cantarell —
  the app looks like itself everywhere, which IS the native feel the design
  language sells. The OS stacks remain one fallback deep (`--font` leads
  with "Inter", `--mono` with "JetBrains Mono").
- **The scale:** 13 px body, 12 px secondary and meta, 11 px micro
  (chips, badges); weights 400 body / 500 labels / 600 emphasis. Larger
  sizes exist only on identity surfaces (see the ladder below).
- **Figures are tabular.** Progress, speed and size columns set
  `font-variant-numeric: tabular-nums` — mid-transfer updates never jitter.

## Iconography

- In-app icons are the hand-drawn stroke set (`components/icons.tsx`): 2 px
  stroke on a 24-unit grid, `currentColor`, round caps and joins, optically
  sized for 14–18 px UI use — ~2 KB of path data instead of an icon package.
- The installer icon set (`src-tauri/icons/`) is GENERATED from the
  platform tile (`npx tauri icon design/logo-tile.svg`) — the set is never
  hand-edited; the masters are the only source.

## Logomark (#79)

The mark is the **dart**: two smooth concentric arcs on one bow — the main
ribbon and its shorter echo — round caps everywhere, zero pointed vertices.
One gesture, one ink: motion without a single hard corner. It was hand-drawn
as SVG paths (set03, owner-approved on the #78 review) and is carried
byte-frozen into every surface; flat solid ink only — color blends are
banned outright by owner ruling (they were a reference-image crutch, #78).

- **Masters:** `design/logo.svg` (bare ink dart for light surfaces) and
  `design/logo-white.svg` (white mono twin for dark surfaces) — 512-unit
  grid, two `<path>` arcs, round caps, no tile, no text, no raster, no
  blends. `design/logo-tile.svg` (near-white rounded tile carrying the ink
  dart) is the platform-icon source where the mark needs a self-contained
  surface (taskbar, dock, installer).
- **One ink, two twins:** `#0F172A` on light surfaces, `#FFFFFF` on dark.
  In the shell the mark paints with `currentColor` so it follows the theme
  automatically (the same rule as the icon set below); README swaps the
  twins with a `<picture>` light/dark source.
- **Surfaces:** the in-app `Logomark` component (`icons.tsx`, byte-frozen
  512-grid paths), `ium-app/public/favicon.svg`, the README lockup, and the
  full Tauri platform set (PNG/ICO/ICNS) regenerated from the tile.
- **Legibility bar:** the twin arcs must read at 32 px taskbar and 512 px
  installer size — strokes stay fat (46/54 units on the frozen grid), so
  nothing thins out down the ladder (verified 96/48/24 on the #78 sheet).

## Logo scale ladder

**The mark scales with the identity weight of the surface (#86, owner
ruling).** Where IUM introduces itself, the dart is BIG; where the user's
work is the hero, the mark steps back or steps away entirely. A logo at one
frozen size is a web-app habit — a desktop product treats its mark as a
system of sizes, each rung chosen for the job that surface does.

| Rung | Size |
| --- | --- |
| `Launch` | 96 px |
| `About` | 64 px |
| `First run and empty states` | 48 px |
| `Chrome` | none |
| `README and docs` | 128 px |

- **Launch (96 px):** the launch/boot surface is THE identity moment — the
  first pixels the product ever shows. The dart at 96 px, the wordmark
  under it, the boot sentence in text-2. Nothing else competes.
- **About (64 px):** Settings → About is the product introducing itself
  again — the mark stands at 64 px above the version facts.
- **First run and empty states (48 px):** an empty library is IUM saying
  "there is nothing here yet — start something"; the mark anchors that
  invitation at 48 px.
- **Chrome (none):** the working chrome carries NO mark. Once IUM is
  introduced, repetition is noise — the OS taskbar/dock icon owns ambient
  identity, and the content owns the window. This is the big-tech rule:
  the toolbar is for commands, not branding.
- **README and docs (128 px):** the README lockup and doc headers render
  the mark at 128 px with the wordmark beside it — print-scale presence
  where the project meets the public.

Between rungs, nothing: the ladder is exhaustive, and `brand.test.ts`
pins the table. A new surface asks "which rung am I?" — if the answer is
"none of them", the surface is redesigning identity and must amend this
sheet first.

## What this sheet governs

All user-visible IUM copy (UI sentences, toasts, empty states, release
notes' product claims) and the name's written forms. Enforcement is
mechanical, in `ium-app/src/brand.test.ts`:

1. this sheet exists with its sections (the #66 acceptance bar);
2. the color table equals `theme.css` — the sheet cannot drift from the CSS;
3. the logo ladder table is present at its exact rungs (the #86 sizing
   law: identity surfaces go bigger, chrome goes without);
4. the copy surface (`copy.ts` rules + default + the app sentences and
   labels) follows the voice: capitalized sentences, no exclamation marks,
   no emoji, the name spelled IUM, adapter labels free of raw seam ids.
   Status-pill texts and button labels are the documented exempt class —
   they are labels, not sentences.
