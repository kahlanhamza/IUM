"""Lane 6 follow-up (#73 CI verdict + the CRLF fix): session-log row."""

row = (
    "| 2026-11-06 (M11#6 CI verdict + the CRLF fix, refs #73) "
    "| **CI caught what the Linux gates could not — and the loop closed it.** "
    "Push `61f3764..d98f36b`; #73 auto-closed on the Closes-keyword; CI run 42277434539: "
    "7/8 GREEN, **ium-app (windows-latest) RED** at vitest — `tokens missing from the sheet "
    "table: [--bg, --surface, …(20)]`. Root cause (log-confirmed): the repo has NO "
    "`.gitattributes`, windows runners check out CRLF, and `sheetTable()` anchors rows on "
    "`$` — a trailing `\\r` defeats every match, the 22-token table parses as empty, both "
    "D2 tests fail on windows only. THE FIX: both file readers normalize CRLF before "
    "parsing (one point per reader); voice tests unaffected (string literals, not "
    "line-anchored). Evidence comment 6693330328 on #73. Follow-up filed per the "
    "no-drive-by rule: **#74** pins `.gitattributes` (`* text=auto eol=lf` + binary "
    "exceptions) so the whole class dies — JSON tables, YAML, markdown tables, bash "
    "harnesses. Wire-facts lesson (on the issue too): vitest stubs `.css` imports to \"\" "
    "even with `?raw`; the gate reads real files via node:fs "
    "| 9/9 local after fix; CI re-run pending on the fix commit "
    "| fix commit |"
)

p = "STATUS.md"
s = open(p).read()
anchor = "| 2026-11-06 (M11#6 — the brand sheet, #73) |"
assert anchor in s, "session-log anchor missing"
s = s.replace(anchor, row + "\n" + anchor, 1)
open(p, "w").write(s)
print("session-log row inserted")
