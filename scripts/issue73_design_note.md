<!--
The design note for #73, archived in-repo (the lane-5 convention):
this file is the issue body as filed. RED evidence:
scripts/issue73_red_evidence.md. Gate: ium-app/src/brand.test.ts.
-->

# Lane 6 (#66): the brand sheet — voice, wordmark, naming rules

Product bar #66, lane 6, from the audit (#8 Branding — MINIMAL): *"a one-page
brand sheet (name, wordmark, voice, color/type rules sourced from theme.css)
— small, but it is what makes the product feel decided."* The acceptance bar:
**a brand sheet exists in-repo and UI copy follows it.**

## Wire facts (all executed in the tree today, none assumed)

- **W1 — the name's official expansion exists only in the product header.**
  `ium-app/src/App.tsx:137-138` renders the wordmark: brand chip (the
  stroke-built `IconUpload` glyph) + **IUM** + sub **"Internet Upload
  Manager"**. No other document in the repo defines the expansion; the
  engine (`ium-core`) and adapters (`adapter_terabox`) never claim it.
- **W2 — the voice is real but unwritten.** The error-copy table (#70,
  `ium-app/src/copy.ts`) already encodes a consistent voice in 18 rules:
  honest ("the engine's own words are below"), specific (what failed + what
  to do), no blame ("the protocol's limit, not a bug"), the engine does the
  waiting ("no action needed"), raw truth never hidden (`raw` stays in view).
  It exists as code and tests — no human-readable rule anywhere.
- **W3 — naming rules exist as code but unwritten.** `logic.ts:47`
  `ADAPTER_LABELS`: engine seams stay technical (`http_oneshot`, `feishu_drive`,
  `s3_multipart`), the UI shows friendly labels ("Feishu Drive", "TeraBox");
  `friendlyAdapter` falls through to the raw id for unknown lanes — the UI
  reflects the engine, it never invents lanes.
- **W4 — the visual identity is 59 CSS custom properties, documented
  nowhere outside the CSS.** `theme.css` carries the full token set: the
  confident teal accent (#0f766e light / #2dd4bf dark, "used sparingly"),
  semantic ok/danger/warn/info pairs, the neutral surface ladder, the
  system-UI type stack + mono for technical strings, 12/8/999 geometry,
  150 ms motion with reduced-motion respect.
- **W5 — nothing machine-checkable guards the acceptance bar today.** A
  sheet could drift from the CSS the day after it is written; UI copy could
  drift from the voice with no gate.

## Design

- **D1 — `docs/10_Brand_Sheet.md`, one page, eight sections:** what IUM is;
  name + wordmark (IUM always uppercase; "Internet Upload Manager" is the
  expansion; the mark = the upload glyph in the brand chip); voice (the
  principles above, with real copy-table examples); naming rules (product
  surfaces friendly, engine seams technical, IUM vs ium-core usage); color
  (the token table, sourced from theme.css); type (system stack + the mono
  rule); iconography (stroke-built in-app icons; the installer icon set);
  what the sheet governs + how the acceptance bar is enforced.
- **D2 — the sheet cannot drift from the CSS.** `brand.test.ts` parses the
  sheet's color table AND theme.css's `:root` + dark block, and requires
  equality — the sheet is a projection of the tokens, not a parallel truth.
- **D3 — the mechanical voice gate (`ium-app/src/brand.test.ts`):** (a) the
  sheet exists with the required sections; (b) the token table matches the
  CSS (D2); (c) UI copy follows the voice — every `copy.ts` title/hint, the
  App toasts and empty-state strings: sentences start capitalized, **no
  exclamation marks** (calm, not shouty), **no emoji** (structured badges
  replaced emoji in #65 and stay replaced), the name is spelled **IUM**
  everywhere it appears, and the adapter label table carries no raw
  underscore ids. Buttons and status-pill labels are a documented exempt
  class (labels, not sentences).
- **D4 — RED.** RED-1: `brand.test.ts` fails at HEAD — the sheet does not
  exist. If the compliance asserts (c) surface real copy violations once the
  sheet exists, those are caught defects, fixed in this issue.

## Gates

`gate.sh` on the committed tree: core lanes (fmt/clippy/tests — untouched)
+ app lanes (vitest incl. `brand.test.ts`, tsc strict, vite build). CI owns
the shell-compile proof (no GTK, the standing contract).
