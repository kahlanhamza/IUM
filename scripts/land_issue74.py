"""Post the LANDED evidence comment on #74."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## LANDED — the pin shipped, CI-green on all three OSes (#74 complete)

**The loop, on the record:**

- **RED (comment 6699305281, at HEAD `633f4f6`)**: `git clone -c core.autocrlf=true` byte-diverges **287 / 307 tracked files**; the `$`-anchored sheetTable parses **0 rows** from the CRLF copy — the lane-6 failure reproduced against HEAD, not recalled from the incident. Wire facts fresh: no `.gitattributes`; 287 text blobs all `i/lf`; binaries = 19 icon assets; `scripts/issue71.json` is 0 bytes.
- **The pin (one commit, `c170ded`)**: `.gitattributes` = `* text=auto eol=lf` (store and check out LF everywhere, regardless of `core.autocrlf`) + explicit `-text` exceptions — the 19 tracked binaries (`*.png` ×17, `icon.ico`, `icon.icns`) and the installer/archive extensions the #65 release bar will produce. **`git add --renormalize .` changed ZERO blobs** — every text file was already stored LF, so the pin changes the checkout contract, not the content.
- **The gates stay robust either way** (this issue's acceptance line): the brand gate and the telemetry charter gate each gained a windows-default checkout simulation — the REAL files converted to CRLF disk bytes, run through the readers' normalization, parsed by the REAL parsers, asserted identical (`ium-app/src/brand.test.ts`, `ium-app/src/telemetry.test.ts`); the Rust charter harness gained the same case (`charter_table_survives_a_crlf_checkout`).
- **Gate honesty note**: the first gate run FAILED on `cargo fmt --check` (3 style violations in the new Rust test) — gate.sh runs on the COMMITTED tree, so the fix was formatted and amended (`79032bc` → `c170ded`) before any push. What got pushed is what got tested.
- **CI run 42317366839 on `c170ded`: 8/8 GREEN, all three OSes** — `ium-app (windows-latest)` now checks out LF under the pin; app-e2e green.

**Gates:** standalone harness **12/12** (11 + the CRLF case); vitest **62/62** (60 + 2 CRLF simulations); tsc strict; vite build 189.71 KB JS; fmt clean; gate.sh GREEN on the committed tree; CI 8/8.

Product-bar #66 lanes 1–7 complete (#67/#69/#70/#71/#72/#73/#75) + this repo-wide hardening (#74); remaining: lane 8 (RE-resistance checklist at release).
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/74/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("landing comment posted:", c["id"])
