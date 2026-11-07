#!/usr/bin/env python3
"""Post the set04 review comment on draft PR #78 and verify raw links."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDRS = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}
RAW = "https://raw.githubusercontent.com/kahlanhamza/IUM/design/logo-concepts/design/logo-variants/set04"

body = """## set04 — the dart, FLAT (zero gradients)

Your ruling applied: **gradients are gone for good** — they were a crutch from the
reference image, not the design. The dart you approved is untouched: the same two
smooth concentric-arc ribbons, byte-for-byte frozen geometry, round caps, no
pointed edges. Only the inks changed — every stroke is now **one solid flat color**.

Pick a colorway:

| # | Colorway | Character |
|---|----------|-----------|
| 1 | **cobalt — one ink** (`#2563EB`) | The infrastructure-grade single color. Reads instantly, scales everywhere, zero decoration. |
| 2 | **duo cyan** (outer `#22D3EE`, inner `#2563EB`) | Keeps the two-tone energy from the reference, but flat. The cyan tip makes the "launch" motion read. |
| 3 | **duo violet** (inner `#7C3AED`, outer `#2563EB`) | Keeps the violet echo, flat. Slightly more premium/odd than cyan. |
| 4 | **ink — stealth** (`#0F172A`) | Near-black for light surfaces; the paper-print option. |
| + | **white mono** | The dark-surface / favicon fallback — already part of any pick. |

![set04 review sheet]({RAW}/showcase_set04_dart_flat.png)

The sheet checks every candidate on **light and dark surfaces** and shows the two
leading candidates at **96 / 48 / 24 px** — the mark survives the ladder because
nothing thins out: strokes stay 46/54 units wide with round caps.

Individual 512 px renders, if you want a clean look at one:

- [cobalt]({RAW}/png/flat_cobalt_512.png) ·
  [duo cyan]({RAW}/png/flat_duo_cyan_512.png) ·
  [duo violet]({RAW}/png/flat_duo_violet_512.png) ·
  [ink]({RAW}/png/flat_ink_512.png) ·
  [white mono]({RAW}/png/mono_white_512.png)

Hand-written SVGs (no defs, no blend tags — grep-verified) live in
`design/logo-variants/set04/svg/`. Docs stay frozen until you pick; the moment you
do, the chosen inks go into the app shell, favicon, and README in one pass.
"""

body = body.replace("{RAW}", RAW)
data = json.dumps({"body": body}).encode()
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/78/comments",
    data=data, headers=HDRS, method="POST")
resp = json.load(urllib.request.urlopen(req))
print("comment posted:", resp["html_url"])

# verify the raw links the owner will click
for f in ["showcase_set04_dart_flat.png", "png/flat_cobalt_512.png",
          "png/flat_duo_cyan_512.png", "png/flat_duo_violet_512.png",
          "png/flat_ink_512.png", "png/mono_white_512.png"]:
    u = f"{RAW}/{f}"
    code = urllib.request.urlopen(urllib.request.Request(u, headers=HDRS)).status
    assert code == 200, (u, code)
print("all 6 raw links verified live")
