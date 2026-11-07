#!/usr/bin/env python3
"""File #79: adopt the dart logomark (ink) — design note rides the issue."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDRS = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}

body = """## What

The owner reviewed the design sets on draft PR #78 and ruled:

1. **set03's dart** is the logomark — "Good I like 02 dart".
2. **No gradients, ever** — "don't use gradient colors at all, it's not good, it's just for reference image". Flat solid inks only.
3. The pick from set04: **ink** — "I like ink" (`#0F172A`, the stealth one-ink colorway; white mono is its dark-surface twin).

This issue adopts that mark across every surface. Docs unfreeze **only** for
logo-related surfaces (README mark, brand sheet logomark section, screenshots
re-shot); lane 7 telemetry work and #59/#60 stay parked.

## Design note (the mark, frozen)

- **Geometry (byte-frozen from set03, owner-approved):** two concentric circular
  arcs on one bow — main ribbon `M 148 314 A 170 170 0 0 1 278 150`, stroke 54;
  echo ribbon `M 214 314.4 A 104 104 0 0 1 261.9 227.4`, stroke 46 — on a 512
  grid inside `translate(11 -10.8) scale(1.15)`. Round caps everywhere. **Zero
  pointed vertices** (the owner's construction ruling that produced set03).
- **Inks:** `#0F172A` on light surfaces; `#FFFFFF` mono on dark. In the shell
  the component rides `currentColor` so the mark follows the theme exactly like
  the sheet's iconography rule (the octicon school) instead of hardcoding hex.
- **No gradient, no blend, no defs in the mark** — grep-verifiable.

## Surfaces to change

| Surface | Change |
|---|---|
| `design/logo.svg` | master = transparent ink dart (replaces the teal tile) |
| `design/logo-tile.svg` | NEW — light rounded tile + ink dart, source for platform icon sets |
| `ium-app/public/favicon.svg` | the ink dart |
| `ium-app/src-tauri/icons/` (17 files) | regenerated from the tile (tauri icon or exact-size renders) |
| `ium-app/src/components/icons.tsx` | `Logomark` = the two dart arcs, currentColor, no rect |
| `ium-app/src/theme.css` | `.logomark` comment follows the component (currentColor, not fixed hex) |
| `README.md` | `<picture>` light/dark swap: ink dart / white mono; alt text; no other prose churn |
| `docs/10_Brand_Sheet.md` | Logomark section rewritten for the dart (geometry, inks, ruling, surfaces) |
| `docs/screenshots/` (6 shots) | re-shot — the old teal tile is in the header of every receipt |

## Gates (this lane's RED → GREEN)

- **RED first:** a new header-gate test pins the dart (exactly two arc paths,
  round linecaps, no `<rect>` tile, aria-hidden, no raster) — fails at HEAD
  where the mark is the teal rect + 3 paths. Evidence on the issue before the fix.
- Then the swap; **existing tests must not weaken**: header quiet-chrome gate,
  brand-voice gate (D2/D3), vitest suite, tsc strict, vite build.
- Core untouched; `cargo fmt + clippy -D warnings + cargo test` still run (the
  standing every-commit bar).

## Non-goals

- No UI token re-theme (`--accent` teal stays; the owner picked a mark, not a palette).
- No CHANGELOG cut (that happens at release), no telemetry, no #59/#60.

Refs: #77 (the design pass this amends), #78 (the review PR, stays open as the
decision record), #66 (product bar).
"""

data = json.dumps({"title": "Adopt the dart logomark — ink colorway, every surface (app, favicon, icons, README, brand sheet, screenshots)", "body": body}).encode()
req = urllib.request.Request("https://api.github.com/repos/kahlanhamza/IUM/issues", data=data, headers=HDRS, method="POST")
issue = json.load(urllib.request.urlopen(req))
print("filed:", issue["number"], issue["html_url"])
