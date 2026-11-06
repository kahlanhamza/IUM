"""Lane 6 (#73): add the CHANGELOG entry + the STATUS.md header row."""

# --- CHANGELOG -------------------------------------------------------------
p = "CHANGELOG.md"
s = open(p).read()
anchor = "## [Unreleased]\n\n### Added\n"
entry = """- The brand sheet: `docs/10_Brand_Sheet.md` records the decided identity —
  the IUM name and wordmark, the voice (honest, specific, calm, the engine's
  truth kept visible), the naming rules (friendly product labels over
  technical seams), the color/type/iconography sourced from the tree — and
  `brand.test.ts` enforces it: the sheet cannot drift from `theme.css`'s
  tokens and the enumerated copy surface follows the voice (#73).
"""
assert anchor in s, "CHANGELOG anchor missing"
s = s.replace(anchor, anchor + entry, 1)
open(p, "w").write(s)
print("CHANGELOG entry added")

# --- STATUS.md header ------------------------------------------------------
row = (
    "(M11#6 LANDED (#73): product-bar lane 6 — the brand sheet. "
    "`docs/10_Brand_Sheet.md` (one page, eight sections: what IUM is; name + wordmark — IUM "
    "always uppercase, expansion \"Internet Upload Manager\" from the README title and the "
    "header sub-line; voice — six principles recorded from the #70 copy table (honest over "
    "reassuring, specific over generic, action over apology, calm — no exclamation marks / no "
    "emoji / no blame, the engine does the work, the journal is the truth); naming rules — "
    "friendly product labels over technical seams (`ADAPTER_LABELS`, unknown ids render raw, "
    "the UI reflects the engine); color — the 22-token projection table; type — the OS stack "
    "is the brand type + mono for technical strings; iconography — the stroke set, 2 px on a "
    "24 grid, currentColor; what the sheet governs). THE GATE (`ium-app/src/brand.test.ts`, "
    "9 tests): (1) the sheet exists with its sections; (2) D2 — the sheet CANNOT drift: the "
    "color table is parsed and required EQUAL to the actual light+dark values in theme.css; "
    "(3) D3 — the enumerated copy surface (18 copy rules + default + APP_COPY + "
    "ADAPTER_LABELS) follows the voice: capitalized sentences, NO exclamation marks, NO "
    "emoji, the name spelled IUM, no raw seam ids in labels. RED: 4/4 ENOENT at HEAD "
    "(`scripts/issue73_red_evidence.md`). Wire-facts lesson en route: vitest stubs `.css` "
    "imports to \"\" even with `?raw` (css:false intercepts first) — the gate reads the real "
    "files via node:fs and `@types/node` rides as a devDep (never ships). APP_COPY + RULES + "
    "DEFAULT_COPY + ADAPTER_LABELS now exported so the voice surface is enumerated; App.tsx "
    "renders the empty state + created toast from the table (byte-identical strings, e2e "
    "seams untouched). Gates: vitest 56/56 (47+9), tsc strict, vite build, bundle "
    "byte-identical; core untouched — gate.sh on the committed tree. Product-bar lanes 1-6 "
    "complete (#67/#69/#70/#71/#72/#73); remaining: lane 7 (telemetry stance = pinned owner "
    "decision, no code before it) / lane 8 (RE-resistance checklist at release)) "
)

p2 = "STATUS.md"
lines = open(p2).read().split("\n")
done = False
for i, l in enumerate(lines):
    if l.startswith("> Last updated: **"):
        prefix = "> Last updated: **"
        lines[i] = prefix + row + l[len(prefix):]
        done = True
        break
assert done, "STATUS header line not found"
open(p2, "w").write("\n".join(lines))
print("STATUS.md header updated")
