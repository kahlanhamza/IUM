#!/usr/bin/env python3
"""Open a draft PR presenting the IUM logomark concept variants for review."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDR = {
    "Authorization": "Bearer " + TOKEN,
    "Accept": "application/vnd.github+json",
    "Content-Type": "application/json",
}
REPO = "kahlanhamza/IUM"
RAW = "https://raw.githubusercontent.com/kahlanhamza/IUM/refs/heads/design/logo-concepts/design/logo-variants"

VARIANTS = [
    ("01_beacon", "Beacon",
     "A ring with one break at the top; the arrow leaves through it. Quiet, one-color, "
     "infrastructure-grade minimal (Vercel / Linear school)."),
    ("02_strata", "Strata",
     "The file stack: the blue bar is the file leaving the queue. Calm, honest, "
     "light-surface craft."),
    ("03_surge", "Surge",
     "Double chevron on a blue gradient circle. Pure momentum; the strongest read at 16 px."),
    ("04_monogram", "Monogram",
     "Custom IU ligature: solid blue I with a 45\u00b0 chamfer cut, stroked ink U on the same "
     "baseline. A signature mark, Stripe / Linear school."),
    ("05_cadence", "Cadence",
     "Ascending rounded bars with a chevron peak \u2014 throughput as a shape."),
    ("06_zephyr", "Zephyr",
     "A 45\u00b0 arrow with a motion trail on a warm orange gradient. Launch energy; "
     "the only diagonal in the set."),
]

rows = "\n".join(
    f'| ![**{i+1:02d} {name}**]({RAW}/png/{key}_512.png) | **{i+1:02d} \u00b7 {name}** | {blurb} |'
    for i, (key, name, blurb) in enumerate(VARIANTS)
)

body = f"""> Design review only \u2014 **do not merge.** `main`, the README and STATUS.md are untouched.
> Once a direction is picked, the chosen mark ships through the normal issue-first loop
> (in-app logomark, favicon, platform icon set, README re-shots).

## The six directions

Each mark is hand-built SVG on a 512 grid, tested at 256 / 96 / 48 / 24 px and on dark.

| Mark | Name | Concept |
|---|---|---|
{rows}

## Full comparison

One page with every variant side by side across sizes:
![contact sheet]({RAW}/contact_sheet.png)

## Files

- `design/logo-variants/contact_sheet.png` \u2014 the full review board
- `design/logo-variants/svg/` \u2014 editable SVG sources
- `design/logo-variants/png/` \u2014 512 px exports
- `scripts/render_logo_variants.py` \u2014 the renderer that produced the board

**Reply with a number** (or a mix, e.g. \u201cMonogram in Beacon\u2019s black\u201d).
"""

payload = json.dumps({
    "title": "Design review: IUM logomark concepts (6 variants) \u2014 do not merge",
    "head": "design/logo-concepts",
    "base": "main",
    "draft": True,
    "body": body,
}).encode()

req = urllib.request.Request(
    f"https://api.github.com/repos/{REPO}/pulls", data=payload, headers=HDR, method="POST")
try:
    d = json.load(urllib.request.urlopen(req))
    print("PR created:", d["number"], d["html_url"], "| draft:", d["draft"])
except urllib.error.HTTPError as e:
    print("HTTPError", e.code, e.read().decode()[:400])
