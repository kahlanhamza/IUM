#!/usr/bin/env python3
"""One-shot: add `concatenation: false,` to every ProbeReport literal in the
test doubles (issue #50 added the field to the struct). Idempotent."""
import re
import sys
from pathlib import Path

ROOT = Path("/home/z/my-project/ium-core")
changed = 0
for path in sorted(ROOT.glob("tests/*.rs")) + sorted(ROOT.glob("src/**/*.rs")):
    text = path.read_text()
    if "ProbeReport {" not in text:
        continue
    out = []
    idx = 0
    modified = False
    while True:
        m = text.find("ProbeReport {", idx)
        if m == -1:
            out.append(text[idx:])
            break
        # find block end: the next line that is only whitespace + '})'
        end = text.find("})", m)
        block = text[m:end]
        if "concatenation" in block:
            idx = end
            continue
        # insert after the last session_token line inside the block
        st = block.rfind("session_token")
        line_end = block.find("\n", st)
        indent = re.match(r"\n?([ \t]*)", block[line_end:]).group(1)
        insertion = f"\n{indent}concatenation: false,"
        new_block = block[:line_end] + insertion + block[line_end:]
        out.append(text[idx:m])
        out.append(new_block)
        idx = end
        modified = True
    if modified:
        path.write_text("".join(out))
        changed += 1
        print(f"patched {path}")
print(f"{changed} file(s) patched")
sys.exit(0)
