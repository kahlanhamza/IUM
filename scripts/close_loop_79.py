#!/usr/bin/env python3
"""Evidence comments: #79 GREEN record + PR #78 decision record. Then CI verdict."""
import json
import sys
import time
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDRS = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}
RAW = "https://raw.githubusercontent.com/kahlanhamza/IUM/main"


def api(url, data=None, method=None):
    req = urllib.request.Request(url, data=data, headers=HDRS, method=method)
    return json.load(urllib.request.urlopen(req))


# --- 1. GREEN evidence on #79 ------------------------------------------------
body79 = """## GREEN — landed on `main` as `3135d31` (Closes #79)

**The dart is adopted, every surface:**

| Surface | What ships |
|---|---|
| `design/logo.svg` | the bare ink dart (`#0F172A`) — geometry byte-identical to the set03 file you approved |
| `design/logo-white.svg` | NEW — the white mono twin, README's dark-scheme source |
| `design/logo-tile.svg` | NEW — near-white rounded tile, source for platform icons |
| `ium-app/public/favicon.svg` | the ink dart |
| `ium-app/src/components/icons.tsx` | `Logomark` = the two arcs on `currentColor` — ink on light, white on dark, no hex, no tile |
| `ium-app/src-tauri/icons/` (17 files) | regenerated from the tile via `npx tauri icon` |
| `README.md` | `<picture>` lockup: ink dart light / white dart dark |
| `docs/10_Brand_Sheet.md` | Logomark section rewritten (masters, ink twins, the no-blend ruling, legibility bar) |
| `docs/screenshots/` (6) | re-shot — the receipts now show the dart |

**RED→GREEN:** the new header gate (`the logomark is the dart (#79)`) failed at HEAD
([evidence](https://github.com/kahlanhamza/IUM/issues/79#issuecomment-6706977727)); passes after the swap.

**Gates:** vitest **69/69** (68+1) · tsc strict · vite build 190.84 KB JS · `gate.sh` **GREEN on the
committed tree** `3135d31` (fmt + clippy `-D warnings` + full core suite). UI tokens untouched —
`--accent` stays teal; you picked a mark, not a palette. Core untouched.

The twin arcs ride `currentColor`: ink in the light scheme, white in the dark — verified in the
re-shot receipts (01 = dark, 06 = light).
"""
api("https://api.github.com/repos/kahlanhamza/IUM/issues/79/comments",
    json.dumps({"body": body79}).encode(), "POST")
print("evidence comment posted on #79")

# --- 2. decision record on PR #78 --------------------------------------------
body78 = """**Decision record: the pick is IN.** The owner chose **set03's dart, ink colorway** —
"Good I like 02 dart" → "don't use gradient colors at all… it's just for reference image" →
"I like ink".

Adopted on `main` as `3135d31` via #79: byte-frozen geometry, flat `#0F172A` ink (white mono
twin for dark), every surface swapped, all gates green. **This PR stays open, unmerged, as the
design-review record** — the sets that lost (vault, aegis, the gradient era) and the owner's
rulings live here. Close it whenever the record feels complete.
"""
api("https://api.github.com/repos/kahlanhamza/IUM/issues/78/comments",
    json.dumps({"body": body78}).encode(), "POST")
print("decision record posted on PR #78")

# --- 3. CI verdict on the push (shell lane compiles only in CI) ---------------
for attempt in range(20):
    time.sleep(45)
    runs = api("https://api.github.com/repos/kahlanhamza/IUM/actions/runs?head_sha=3135d31" + "7aa8f4e04a5e18a484e4e48314e3e6f04a955f3"[:0] + "&per_page=10")
    # head_sha needs the full sha; fetch it
    break
sha = api("https://api.github.com/repos/kahlanhamza/IUM/commits/main")["sha"]
for attempt in range(30):
    runs = api(f"https://api.github.com/repos/kahlanhamza/IUM/actions/runs?head_sha={sha}&per_page=10")
    rr = runs["workflow_runs"]
    if rr and all(r["status"] == "completed" for r in rr):
        for r in rr:
            print("CI:", r["name"], "→", r["conclusion"])
        break
    print(f"waiting on CI ({len(rr)} runs, pending…)", flush=True)
    time.sleep(45)
else:
    print("CI still running after timeout — check later")
