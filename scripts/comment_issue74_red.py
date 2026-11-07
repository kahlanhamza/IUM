"""Post the RED evidence comment on #74."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## RED evidence — the class exposure, executed at HEAD `633f4f6` (2026-11-07)

`scripts/issue74_red_evidence.sh` simulates the windows-default clone and reads the real tree:

- **`git clone -c core.autocrlf=true` (no pin): 287 / 307 tracked files byte-diverge** from their committed blobs — every text file gains CRLF on disk. The 19 icon binaries and 1 empty file are the only stable paths.
- **The lane-6 failure reproduces from scratch**: the brand sheet's `$`-anchored table regex parses **0 rows** from the CRLF copy (Linux gate parses 22) — the exact run-42277434539 condition, now proven against HEAD rather than recalled from the incident.
- Sanity: the same clone without `core.autocrlf=true` stays LF (0 working-tree CRLF files) — the divergence is the eol config, not the content.

Wire facts executed fresh at `633f4f6`: no `.gitattributes`; 287 text blobs all `i/lf` (renormalize will be a zero-blob-change commit); binaries = 19 icon assets (`*.png` ×17, `icon.ico`, `icon.icns`); `scripts/issue71.json` is `i/none` (0 bytes); parser inventory that depends on byte-stable text: `brand.test.ts` (sheet + theme, readers normalize since `c237b66`), `telemetry.test.ts` + the Rust `charter_table` harness (both tolerant by construction), workflow YAML, the bash harnesses.

Next: the pin (`* text=auto eol=lf` + `-text` binary exceptions), renormalize, CRLF-converted-copy gates on both charter/table pipelines, full gates, CI 3-OS.
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/74/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("RED comment posted:", c["id"])
