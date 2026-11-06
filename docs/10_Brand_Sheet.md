# The IUM brand sheet

**Product bar #66, lane 6 (#73).** One page; the decided identity of the
product. Every value here is lifted from the tree — `theme.css`, `copy.ts`,
`logic.ts`, `App.tsx` — nothing aspirational. `ium-app/src/brand.test.ts`
enforces it: the sheet cannot drift from the tokens it documents, and the
UI copy must follow the voice it declares.

## What IUM is

An upload manager that applies download-manager reliability engineering to
uploads: resumable, crash-safe, verified before it says Done. The brand
consequence is the whole voice: **the product speaks the engine's truth.**
It reports what happened, never what sounds nice.

## Name and wordmark

- The name is **IUM** — three letters, always uppercase, never "Ium".
- The expansion is **Internet Upload Manager** — the README title's form and
  the header's sub-line (`App.tsx`); no other tagline exists or is invented.
- The wordmark is one lockup: the stroke-built upload glyph (`IconUpload`)
  in the accent brand chip, **IUM** beside it, the expansion as the sub-line.
  No other lockups, no logomark, no word-on-shape variants.
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
| `tus` | TUS resumable |
| `s3_multipart` | S3 multipart |
| `feishu_drive` | Feishu Drive |
| `terabox` | TeraBox |
| `mega` | MEGA |

- Unknown adapter ids render raw (`friendlyAdapter` falls through) — the UI
  reflects the engine; it never invents lanes.
- `adapter_terabox`, `JobsTable`, `ChunkPool` and friends stay verbatim in
  code and engineering docs; the friendly label is a UI surface only.

## Color

The accent is **one confident teal, used sparingly** — theme.css's own
comment. Semantic colors always travel as a foreground/background pair.
The table is the projection of the tokens; `brand.test.ts` requires exact
equality with `theme.css` in both schemes.

| token | light | dark |
| --- | --- | --- |
| `--bg` | `#f5f6f8` | `#131518` |
| `--surface` | `#ffffff` | `#1c1f23` |
| `--surface-2` | `#eef0f3` | `#24282d` |
| `--surface-3` | `#e6e9ed` | `#2d3238` |
| `--text` | `#191c1f` | `#e9ebee` |
| `--text-2` | `#5b6470` | `#a6adb7` |
| `--text-3` | `#8b93a0` | `#727a86` |
| `--border` | `#e2e5ea` | `#2b2f35` |
| `--border-strong` | `#ccd2d9` | `#3d434b` |
| `--accent` | `#0f766e` | `#2dd4bf` |
| `--accent-hover` | `#0d6963` | `#43dcc9` |
| `--accent-down` | `#0b5a54` | `#5ae3d2` |
| `--accent-text` | `#ffffff` | `#062823` |
| `--accent-subtle` | `#e4f4f2` | `#123f3a` |
| `--ok` | `#15803d` | `#4ade80` |
| `--ok-bg` | `#e6f5ec` | `#12331f` |
| `--danger` | `#b91c1c` | `#f87171` |
| `--danger-bg` | `#fdeceb` | `#3b1515` |
| `--warn` | `#92580a` | `#fbbf24` |
| `--warn-bg` | `#fcf1dd` | `#3a2a10` |
| `--info` | `#1d4ed8` | `#93b4fd` |
| `--info-bg` | `#e8eefc` | `#1a2745` |

The focus halo (`--accent-ring`) is a rgba value on purpose — it is motion
state, not a brand color, and lives in `theme.css` only. Geometry (12 px
cards, 8 px controls, pill radius) and motion (150 ms, reduced-motion
respect) are theme.css territory; the sheet records the fact, not the numbers.

## Type

- **The OS's own type is the brand type.** UI text rides the system stack
  (`--font`: system-ui, -apple-system, "Segoe UI Variable", …) — no brand
  font is shipped or needed; the lightweight bar applies to type too.
- **Technical strings ride mono.** Endpoints, adapter ids, byte counts and
  the "the engine said: …" raw line use `--mono` — the seam between product
  and engine is also a typographic one.

## Iconography

- In-app icons are the hand-drawn stroke set (`components/icons.tsx`): 2 px
  stroke on a 24-unit grid, `currentColor`, round caps and joins, optically
  sized for 14–18 px UI use — ~2 KB of path data instead of an icon package.
- The installer icon set (`src-tauri/icons/`) ships the per-OS marks; a
  custom mark, if ever wanted, replaces that set — the wordmark lockup does
  not change to match it.

## What this sheet governs

All user-visible IUM copy (UI sentences, toasts, empty states, release
notes' product claims) and the name's written forms. Enforcement is
mechanical, in `ium-app/src/brand.test.ts`:

1. this sheet exists with its sections (the #66 acceptance bar);
2. the color table equals `theme.css` — the sheet cannot drift from the CSS;
3. the copy surface (`copy.ts` rules + default + the app sentences and
   labels) follows the voice: capitalized sentences, no exclamation marks,
   no emoji, the name spelled IUM, adapter labels free of raw seam ids.
   Status-pill texts and button labels are the documented exempt class —
   they are labels, not sentences.
