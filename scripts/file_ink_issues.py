#!/usr/bin/env python3
"""File the Ink-program issues (#86 palette LANDED, #87 launch window, #88 anatomy)
once the PAT is restored (.secrets/gh_token — wiped by recycle #14, 2026-11-08).

Idempotent: detects an existing open issue by exact title and skips it.
Run: python3 scripts/file_ink_issues.py
"""

import json
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()

BODY_86 = """## The Ink palette — two inks, two schemes, one type system, the logo ladder

**Owner-approved direction (the Ink mockups) + the owner's sizing law:**
*"you must know when to increase the size of it and where to decrease it —
in some pages where identity is relevant or where we introduce our software
to users the logo must be bigger."*

### Design note

D1. **Two inks.** The brand color is the logomark's ink — `#0F172A` — and
its inverted twin (near-white) on dark. Light mode speaks the dark ink on
paper white; dark mode speaks the light ink on midnight chrome. The accent
IS the ink: filled controls are ink with the scheme's opposite type. Teal
is banned (owner ruling). Red speaks ONLY failure; green speaks engine
health; Done/paused go neutral ink (color reserved for meaning).
D2. **Type is the interface.** Inter 400/500/600 (UI) + JetBrains Mono
400/500 (machine data), vendored latin-subset woff2 (~140 KB, OFL licenses
beside the files). Vendoring is the cross-OS guarantee: macOS, Windows and
Linux render the SAME faces. 13 px body scale; tabular numerals on figures.
D3. **The theme is a first-class mode pair.** Light = `:root` base (no
attribute — a pre-paint frame can never flash); dark = `[data-theme="dark"]`.
The preference (system/light/dark) is a UI concern: localStorage, applied
before first paint, live-follows the OS while `system`.
D4. **The logo ladder (the sizing law, mechanically pinned).** Launch 96 px /
About 64 px / First-run+empty 48 px / Chrome none / README+docs 128 px.
A new surface must ask which rung it is; between rungs, nothing.
D5. **Accessibility is a gate, not a wish.** Every text/background pair in
both schemes ≥ 4.5:1 (WCAG AA), enforced by the audit script; the first cut
FAILED five pairs and the tokens were retuned, not the bar lowered.
D6. **Cross-platform note.** The Ink language is one system on all three
OSes (the VS Code/Slack/Linear pattern); window frames stay native per OS.
The vendored fonts remove the SF/Segoe/Cantarell drift — the app looks like
itself everywhere.

### RED (captured at HEAD before the swap)

8 failing pins: the ladder section + table (absent), the five @font-face
declarations (absent), the vendored woff2 on disk (ENOENT), the font stacks
(system-ui led), the dark attribute block (media query only), and the theme
module itself (import failed). Full output posted below as a comment.

### Implementation

- `theme.css`: the two-ink token system + @font-face × 5 + 13 px base +
  tabular figures + the color-discipline chip/pill changes.
- `theme.ts` (pure: load/save/resolve/apply/watch) + `ThemeToggle.tsx`
  (real radiogroup) + the Appearance card leading Settings.
- `main.tsx`: `initTheme()` before first render.
- `docs/10_Brand_Sheet.md`: the Color/Type sections rewritten; the new
  **Logo scale ladder** section (pinned table); governance list updated.
- `brand.test.ts`: the new pins; the token projection now splits on the
  line-start `[data-theme=\"dark\"]` selector (the design-law comment
  mentions the attribute mid-sentence — the parse had to be exact).
- `scripts/ink_contrast_audit.py`: the WCAG audit (FAILING first cut kept
  honest in the evidence comment).
- `public/fonts/`: 5 woff2 + 2 OFL licenses. `.gitattributes` already pins
  `*.woff2 -text` (#74).
- The six README screenshots re-shot through the real harness — the
  receipts show the Ink app.

### Gates

vitest **114/114** (13 files; was 97) · tsc strict · vite build 204.87 KB
JS / 63.57 KB gzip (fonts ride as static assets) · cargo fmt + clippy
`-D warnings` + **313/313** · WCAG AA both schemes (audit clean).

Closes the palette lane of the Ink program. The launch window (#87) and the
chrome anatomy (#88) follow.
"""

BODY_87 = """## The launch window — identity at first paint (the logo ladder's 96 px rung)

**Owner question (answered by this lane):** *"what about launching page, is
there one?"* — There is not: `tauri.conf.json` declares one window and the
app's first paint is the working chrome. This lane builds the launch surface
the approved mockup (`design/mockup/launch.html`, `ium-ui-mock-6`) shows.

### Design note

D1. **One identity moment.** A small frameless centered window (Tauri
multi-window, uniform on macOS/Windows/Linux): the dart at **96 px** (the
ladder's Launch rung — the owner's sizing law), the wordmark under it, one
boot sentence in text-2. Nothing else competes.
D2. **Honest boot.** The sentence reports the real boot phases (starting
the engine → connecting → ready). It never fakes progress; a slow boot is
VISIBLE, not animated away.
D3. **The handoff.** The main window stays hidden until the shell signals
ready (engine status resolved or a hard timeout — a down engine must not
trap the user at the splash; the main UI owns the "engine unreachable"
state honestly). Splash closes, main reveals: no double-draw, no flash.
D4. **Cross-platform.** Frameless centered windows work identically on all
three OSes; the main window keeps its NATIVE frame (traffic lights /
Win11 controls / DE decorations) — the Ink language is content, the frame
stays the OS's own until the P3 materials pass.
D5. **Performance.** The splash is a static SVG + text — no animation
library, no timers beyond the handoff timeout, no backdrop filters.

### Acceptance

- `tauri.conf.json` carries the splash window (label `splash`, frameless,
  centered, non-resizable) and the main window starts hidden.
- The splash page renders the Logomark at exactly 96 px (ladder-pinned) +
  wordmark + one live boot sentence; pins in a new test file.
- The handoff emits on ready or the timeout fires, whichever first; both
  paths covered by a test.
- e2e lane unaffected (the locator inventory unchanged; the splash path
  exercised by the component tests, the WebDriver lane drives the main
  window after handoff).
- Gates: vitest green, tsc + build, cargo fmt/clippy/test, a11y audit.

Closes #86's program order: palette (landed) → **launch** → anatomy (#88).
"""

BODY_88 = """## The Ink chrome — the anatomy pass (toolbar, sidebar, dense rows, status bar, docked inspector, modal settings)

The owner-approved mockups (`design/mockup/*.html`, shots 1–5) define the
chrome the palette lane now sits under. This lane moves the real app onto
that anatomy.

### Design note

D1. **Anatomy over decoration.** 48 px toolbar (window commands + quiet
status, NO brand block — the ladder's Chrome rung: none), 210 px sidebar
(Library: All/Active/Queued/Completed/Failed + Destinations with
connection dots), edge-to-edge 36 px rows with hairline separators and
right-aligned tabular figures, 28 px status bar (engine · address ·
version), the docked inspector (push, never overlay), settings as a
centered 640 px modal over the smoke scrim with the Esc affordance.
D2. **The four-pane contract.** Command (toolbar) / navigate (sidebar) /
work (rows) / explain (inspector + status bar) — the Transmit/ForkLift/
WinSCP lineage, the same anatomy Linear and Raycast ship.
D3. **Density is the native feel.** 36 px rows, hairline dividers, no
floating cards, no drop shadows inside the window (hairlines carry every
edge; shadows only under the floating modal).
D4. **e2e and a11y in lockstep.** The locator inventory is amended in the
same commit as any seam move; the axe gate re-run on the new anatomy; the
README screenshots re-shot at the landing (the receipts bar).
D5. **Performance contract.** Zero new JS dependencies; animations ≤ 150 ms
opacity/2 px only; no backdrop-filter.

### Acceptance

- The toolbar/sidebar/rows/status-bar/docked-inspector/modal-settings
  anatomy renders in the real app at the mockup's metrics.
- The brand block is GONE from the working chrome (the ladder's Chrome
  rung enforced in the component, not only the sheet).
- JobsTable rows are the dense edge-to-edge list; the inspector docks and
  pushes; settings is the centered modal (the drawer is retired).
- Locator inventory + e2e lane updated in the same commit; a11y clean.
- Gates: vitest green, tsc + build, cargo suite green, screenshots re-shot.

Closes the Ink program.
"""

ISSUES = [
    {
        "title": "The Ink palette — two inks, two schemes, one type system, the logo ladder",
        "body": BODY_86,
        "labels": ["process"],
        "state": "open",  # filed open, closed by the evidence comment (commit predates the PAT return)
    },
    {
        "title": "The launch window — identity at first paint (the 96 px rung)",
        "body": BODY_87,
        "labels": ["process"],
    },
    {
        "title": "The Ink chrome — the anatomy pass (toolbar, sidebar, dense rows, status bar)",
        "body": BODY_88,
        "labels": ["process"],
    },
]


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        data=json.dumps(payload).encode() if payload else None,
        headers={
            "Authorization": f"token {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def main() -> None:
    existing = api("GET", "issues?state=all&per_page=100")
    titles = {i["title"]: i for i in existing}
    for spec in ISSUES:
        if spec["title"] in titles:
            print(f"skip (exists as #{titles[spec['title']]['number']}): {spec['title']}")
            continue
        out = api("POST", "issues", {
            "title": spec["title"],
            "body": spec["body"],
            "labels": spec.get("labels", []),
        })
        print(f"filed #{out['number']}: {out['title']}")


if __name__ == "__main__":
    main()
