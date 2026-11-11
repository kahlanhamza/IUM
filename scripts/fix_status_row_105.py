#!/usr/bin/env python3
"""Move the #105 row: Done-table (wrong) -> session log after '## 9. Session log' separator."""
P = "STATUS.md"

with open(P, encoding="utf-8") as f:
    lines = f.readlines()

row = None
kept = []
for line in lines:
    if "the pre-release accuracy sweep, Closes #105" in line and line.startswith("| 2026-11-11"):
        row = line
        continue
    kept.append(line)
assert row is not None, "row not found"

# find the session-log separator AFTER the '## 9. Session log' header
hdr = next(i for i, l in enumerate(kept) if l.startswith("## 9. Session log"))
sep = next(i for i in range(hdr, len(kept)) if kept[i].startswith("|---|---|---|---|"))
kept.insert(sep + 1, row)

with open(P, "w", encoding="utf-8") as f:
    f.writelines(kept)
print(f"row moved: was in Done table, now after session-log separator at line {sep + 2}")
