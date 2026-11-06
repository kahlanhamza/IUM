"""Lane 6 (#73): insert the Done-table row + the session-log row."""
import re

done_row = (
    "| 2026-11-06 (M11#6 LANDED — the brand sheet, #73) "
    "| **Lane 6 of #66, full loop — the product's decided identity, made ENFORCED.** "
    "Wire facts FIRST (executed in the tree, none assumed): W1 the name's official expansion "
    "lives only in the product header (`App.tsx:137-138` — IUM + \"Internet Upload Manager\", "
    "matching the README title); W2 the voice is real but unwritten — the #70 copy table "
    "encodes it in 18 rules (honest, specific, action-bearing, calm, engine-does-the-work, "
    "raw truth visible); W3 the naming rules exist as code but unwritten (`ADAPTER_LABELS` "
    "six friendly labels, `friendlyAdapter` falls through raw — the UI reflects the engine, "
    "it never invents lanes); W4 the visual identity is 59 CSS custom properties documented "
    "nowhere outside the CSS; W5 nothing machine-checkable guarded the #66 acceptance bar. "
    "Design D1-D4 (`scripts/issue73_design_note.md`, the issue body). RED: 4/4 ENOENT at "
    "HEAD (`scripts/issue73_red_evidence.md`). LANDED: (a) **`docs/10_Brand_Sheet.md`** — "
    "one page, eight sections, every value lifted from the tree; (b) **the drift guard** — "
    "the sheet's 22-token color table is parsed and required EQUAL to theme.css's actual "
    "light+dark values (the sheet is a projection of the CSS, not a parallel truth); "
    "(c) **the voice gate** — the enumerated copy surface (copy rules + default + APP_COPY + "
    "ADAPTER_LABELS) asserted: capitalized sentences, no exclamation marks, no emoji, the "
    "name spelled IUM, no raw seam ids in labels; buttons/status pills the documented exempt "
    "class. Copy-surface consolidation en route: APP_COPY (created toast + empty state) now "
    "lives in copy.ts and App.tsx renders from the table — byte-identical strings, e2e "
    "seams untouched. Wire-facts lesson: vitest stubs `.css` imports to \"\" even with "
    "`?raw` — the gate reads the real files via node:fs (@types/node = devDep, never "
    "ships). Gates: vitest **56/56** (47+9), tsc strict, vite build, bundle byte-identical "
    "(test files don't ship); core untouched; gate.sh on the committed tree; shell compile "
    "proof = CI (no GTK, the standing contract) "
    "| 56/56; 9/9 brand; RED 4/4 ENOENT; issue filed before push "
    "| this commit |"
)

log_row = (
    "| 2026-11-06 (M11#6 — the brand sheet, #73) "
    "| **Lane 6 of #66, full loop.** Opened with the lane-5 tail the dead session left: "
    "the token returned, `.secrets/` restored (gh_token + mega.json; TeraBox files not "
    "needed this session), the unpushed UUID-messaged lane-5 commit verified by execution "
    "(275/275 twice + gate.sh GREEN), issue #72 filed from the prepared payload, evidence "
    "comments posted, message amended, pushed — CI 8/8 GREEN incl. app-e2e, #72 auto-closed "
    "(the closing keyword is part of the deliverable). Lane 6 then ran clean: owner said "
    "\"Go ahead next\" → lane 7 stays blocked (pinned owner decision), lane 6 = the brand "
    "sheet. Wire facts from the tree (W1-W5); design note filed as the issue body; RED 4/4 "
    "ENOENT; sheet + gate written; the gate's first GREEN run caught the vitest css-stub "
    "fact (?raw returns \"\" for .css) → node:fs + @types/node; tsc caught the unexported "
    "CopyRule. Gates: vitest 56/56, tsc strict, vite build, gate.sh on the committed tree "
    "| 56/56; 9/9 brand; push pending "
    "| this commit |"
)

p = "STATUS.md"
s = open(p).read()

# Done table: insert before the M11#5 row (line 87 area — anchor on the row start)
anchor_done = "| 2026-11-06 (M11#5 LANDED — 100-jobs load e2e + the installer-size budget gate, #72) |"
assert anchor_done in s, "done-table anchor missing"
s = s.replace(anchor_done, done_row + "\n" + anchor_done, 1)

# Session log: insert before the M11#5 session-log row
anchor_log = "| 2026-11-06 (M11#5 — 100-jobs load e2e + the installer-size budget gate, #72) |"
assert anchor_log in s, "session-log anchor missing"
s = s.replace(anchor_log, log_row + "\n" + anchor_log, 1)

open(p, "w").write(s)
print("STATUS.md rows inserted")
