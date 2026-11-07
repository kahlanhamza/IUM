#!/usr/bin/env python3
"""Update PR #78 body: Set 03 (owner-reference direction) on top, earlier sets collapsed."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {"Authorization": "Bearer " + TOKEN, "Accept": "application/vnd.github+json",
       "Content-Type": "application/json"}
REPO = "kahlanhamza/IUM"
RAW03 = "https://raw.githubusercontent.com/kahlanhamza/IUM/refs/heads/design/logo-concepts/design/logo-variants/set03"
RAW01 = "https://raw.githubusercontent.com/kahlanhamza/IUM/refs/heads/design/logo-concepts/design/logo-variants"

body = f"""> Design review only \u2014 **do not merge.** `main`, the README and STATUS.md are untouched.
> **Set 03 follows your reference direction**: hand-drawn smooth geometry \u2014 every corner
> filleted with bezier curves, ribbon gradients (cyan \u2192 blue \u2192 violet), folded planes,
> no pointed edges. Each mark ships in the three states your reference uses:
> color / white monochrome / construction grid with golden circles.

## Set 03 \u00b7 hand-drawn smooth geometry (current)

| Mark | Story |
|---|---|
| ![01 Vault]({RAW03}/png/01_vault_color.png) | **01 \u00b7 Vault** \u2014 the opened crate. Isometric box with a dark opening to the sky; all 12 outer corners filleted by hand (`Q` curves). The crate you fill, uploading upward. |
| ![02 Dart]({RAW03}/png/02_dart_color.png) | **02 \u00b7 Dart** \u2014 twin rising ribbons. Two concentric circular arcs, one gesture: the cyan lead and the violet echo. Pure curves \u2014 not a single straight edge. |
| ![03 Aegis]({RAW03}/png/03_aegis_color.png) | **03 \u00b7 Aegis** \u2014 the hex vessel. A filleted hexagonal band with a darker folded ribbon on the right side and a filleted arrow rising through the middle. |

### The three states (Vault shown; all marks come in all three)

| Color | White monochrome | Construction |
|---|---|---|
| ![color]({RAW03}/png/01_vault_color.png) | ![mono]({RAW03}/png/01_vault_mono.png) | ![construction]({RAW03}/png/01_vault_construction.png) |

### Full board \u2014 all three marks \u00d7 three states

![Set 03 showcase]({RAW03}/showcase_set03.png)

## Earlier sets

<details>
<summary><strong>Set 01 \u00b7 six flat minimal directions</strong> (superseded \u2014 kept for reference)</summary>
![Set 01 contact sheet]({RAW01}/contact_sheet.png)
</details>

<details>
<summary><strong>Set 02 \u00b7 first futuristic attempt</strong> (superseded by Set 03's hand-drawn geometry)</summary>
![Set 02 showcase]({RAW01}/set02/showcase_set02.png)
</details>

## Files

- `design/logo-variants/set03/svg/` \u2014 hand-written SVG masters (color/mono/construction per mark)
- `design/logo-variants/set03/png/` \u2014 512 px exports
- `design/logo-variants/set03/showcase_set03.png` \u2014 the review board
- `scripts/gen_logo_set03.py` \u2014 masters are authored by hand inside it; Python only recolors and composes

**Reply with a number** \u2014 or say what to push further (rounder, bolder, more fold, different gradient).
"""

payload = json.dumps({"body": body}).encode()
req = urllib.request.Request(f"https://api.github.com/repos/{REPO}/pulls/78",
                             data=payload, headers=HDR, method="PATCH")
d = json.load(urllib.request.urlopen(req))
print("PR #78 body updated | title:", d["title"], "| state:", d["state"], "| draft:", d["draft"])

req2 = urllib.request.Request(f"https://api.github.com/repos/{REPO}/pulls/78/files?per_page=100",
                              headers=HDR)
files = json.load(urllib.request.urlopen(req2))
print("files now:", len(files))
