#!/usr/bin/env python3
"""Concept Set 03 — IUM logomarks, hand-drawn smooth geometry.

Every path below is authored by hand: corners are filleted with Q beziers,
wings are pure C beziers, the hex band is a filleted closed path. Python only
re-colors the masters (color / mono / construction) and composes the board.
"""
import os

import cairosvg
from PIL import Image, ImageDraw, ImageFont

BASE = "/home/z/my-project/download/ium_logo_variants"
OUT = os.path.join(BASE, "set03")
os.makedirs(os.path.join(OUT, "svg"), exist_ok=True)
os.makedirs(os.path.join(OUT, "png"), exist_ok=True)

BG = "#05070D"
INK = "#9AA7BA"

# ------------------------------------------------------------------ masters
VAULT = """<svg width="512" height="512" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
<defs>
  <linearGradient id="gTop" gradientUnits="userSpaceOnUse" x1="150" y1="140" x2="360" y2="230">
    <stop offset="0%" stop-color="#7DD3FC"/><stop offset="100%" stop-color="#3B82F6"/>
  </linearGradient>
  <linearGradient id="gLeft" gradientUnits="userSpaceOnUse" x1="126" y1="196" x2="256" y2="391">
    <stop offset="0%" stop-color="#2563EB"/><stop offset="100%" stop-color="#1E3A8A"/>
  </linearGradient>
  <linearGradient id="gRight" gradientUnits="userSpaceOnUse" x1="256" y1="391" x2="386" y2="196">
    <stop offset="0%" stop-color="#3B82F6"/><stop offset="100%" stop-color="#7C3AED"/>
  </linearGradient>
</defs>
<!-- left wall: four filleted corners (r=22) -->
<path class="facet" fill="url(#gLeft)" stroke="url(#gLeft)" stroke-width="16" stroke-linejoin="round"
  d="M 126 287 L 126 218 Q 126 196 145.1 207 L 236.9 260 Q 256 271 256 293
     L 256 369 Q 256 391 236.9 380 L 145.1 320 Q 126 309 126 287 Z"/>
<!-- right wall -->
<path class="facet" fill="url(#gRight)" stroke="url(#gRight)" stroke-width="16" stroke-linejoin="round"
  d="M 386 287 L 386 218 Q 386 196 366.9 207 L 275.1 260 Q 256 271 256 293
     L 256 369 Q 256 391 275.1 380 L 366.9 320 Q 386 309 386 287 Z"/>
<!-- top face: filleted diamond (r=20) -->
<path class="facet" fill="url(#gTop)" stroke="url(#gTop)" stroke-width="16" stroke-linejoin="round"
  d="M 273.3 131 L 368.7 185 Q 386 196 366.9 206 L 273.3 261 Q 256 271 238.7 261
     L 143.3 206 Q 126 196 143.3 186 L 238.7 131 Q 256 121 273.3 131 Z"/>
<!-- the opening: dark filleted diamond (r=12) -->
<path class="dark" d="M 266.4 160.8 L 317.1 190 Q 327.5 196 317.1 202 L 266.4 231.3
  Q 256 237.3 245.6 231.3 L 194.9 202 Q 184.5 196 194.9 190 L 245.6 160.8
  Q 256 154.8 266.4 160.8 Z"/>
<!-- seams where facets abut (mono/construction only) -->
<path class="seam" d="M 143.3 206 L 238.7 260.9"/>
<path class="seam" d="M 273.3 260.9 L 368.7 206"/>
<path class="seam" d="M 256 293 L 256 369"/>
</svg>
"""

DART = """<svg width="512" height="512" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
<defs>
  <linearGradient id="gHi" gradientUnits="userSpaceOnUse" x1="130" y1="340" x2="310" y2="130">
    <stop offset="0%" stop-color="#22D3EE"/><stop offset="100%" stop-color="#2563EB"/>
  </linearGradient>
  <linearGradient id="gLow" gradientUnits="userSpaceOnUse" x1="200" y1="320" x2="270" y2="220">
    <stop offset="0%" stop-color="#7C3AED"/><stop offset="100%" stop-color="#6D28D9"/>
  </linearGradient>
</defs>
<!-- twin ribbons on one bow: concentric circular arcs, one gesture -->
<g transform="translate(11 -10.8) scale(1.15)">
  <!-- inner echo ribbon: violet, shorter span, hugging the same bow -->
  <path class="facet" fill="none" stroke="url(#gLow)" stroke-width="46" stroke-linecap="round"
    d="M 214 314.4 A 104 104 0 0 1 261.9 227.4"/>
  <!-- main ribbon: cyan energy rising along the outer bow -->
  <path class="facet" fill="none" stroke="url(#gHi)" stroke-width="54" stroke-linecap="round"
    d="M 148 314 A 170 170 0 0 1 278 150"/>
</g>
</svg>
"""

AEGIS = """<svg width="512" height="512" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
<defs>
  <linearGradient id="gHex" gradientUnits="userSpaceOnUse" x1="140" y1="140" x2="372" y2="390">
    <stop offset="0%" stop-color="#60A5FA"/><stop offset="100%" stop-color="#7C3AED"/>
  </linearGradient>
  <linearGradient id="gFold" gradientUnits="userSpaceOnUse" x1="386" y1="220" x2="285" y2="395">
    <stop offset="0%" stop-color="#8B5CF6"/><stop offset="100%" stop-color="#6D28D9"/>
  </linearGradient>
  <linearGradient id="gArr" gradientUnits="userSpaceOnUse" x1="256" y1="158" x2="256" y2="350">
    <stop offset="0%" stop-color="#67E8F9"/><stop offset="100%" stop-color="#2563EB"/>
  </linearGradient>
</defs>
<!-- hex band: closed filleted path (r=32), stroked -->
<path class="facet" fill="none" stroke="url(#gHex)" stroke-width="40" stroke-linejoin="round"
  d="M 283.7 138 L 350.7 176 Q 377.2 192 377.2 224 L 377.2 300 Q 377.2 332 350.7 348
     L 283.7 386 Q 256 402 228.3 386 L 162.5 348 Q 134.8 332 134.8 300
     L 134.8 224 Q 134.8 192 162.5 176 L 228.3 138 Q 256 122 283.7 138 Z"/>
<!-- the fold: darker ribbon over the right side, ends at vertices -->
<path class="facet mono-hide" fill="none" stroke="url(#gFold)" stroke-width="40" stroke-linecap="round"
  d="M 377.2 224 L 377.2 300 Q 377.2 332 350.7 348 L 283.7 386"/>
<!-- arrow: filleted head notches (r=12), rounded tip (r=10), rounded shaft (r=12) -->
<path class="facet" fill="url(#gArr)" stroke="url(#gArr)" stroke-width="12" stroke-linejoin="round"
  d="M 237 338 L 237 254 Q 237 242 225 242 L 202 242 Q 190 242 197.4 232.6
     L 249.8 165.9 Q 256 158 262.2 165.9 L 314.6 232.6 Q 322 242 310 242
     L 287 242 Q 275 242 275 254 L 275 338 Q 275 350 263 350 L 249 350
     Q 237 350 237 338 Z"/>
<!-- seam where the fold crosses the band (mono/construction only) -->
<path class="seam" stroke="#05070D" stroke-width="7" stroke-linecap="round" fill="none"
  d="M 377.2 224 L 377.2 300 Q 377.2 332 350.7 348 L 283.7 386"/>
</svg>
"""

MARKS = [
    ("01_vault", "VAULT", "the opened crate \u00b7 every corner filleted", VAULT, (256, 256), False),
    ("02_dart", "DART", "twin rising ribbons \u00b7 one bow, layered motion", DART, (256, 256), False),
    ("03_aegis", "AEGIS", "the hex vessel \u00b7 folded ribbon, rising arrow", AEGIS, (256, 262), True),
]

# ------------------------------------------------------------------ deriver
import re

ATTR = re.compile(r'(class="[^"]*"|fill="[^"]*"|stroke="[^"]*")')


def derive(master, mode, overlay=""):
    """color: everything as authored, seams hidden.
    mono: all gradient paint -> white; fold hidden; seams shown.
    construction: all gradient paint -> ink; seams shown; overlay appended."""
    out = []
    for line in master.splitlines():
        structural = any(t in line for t in ("stop-color", "<defs", "</defs",
                                             "<svg", "</svg>", "linearGradient"))
        if structural:
            out.append(line)
            continue
        if "mono-hide" in line and mode in ("mono", "construction"):
            continue
        if 'class="seam"' in line and mode == "color":
            continue
        if mode == "mono":
            line = re.sub(r'(fill|stroke)="url\(#[^"]+\)"', r'\1="#FFFFFF"', line)
        elif mode == "construction":
            line = re.sub(r'(fill|stroke)="url\(#[^"]+\)"', f'\\1="{INK}"', line)
        out.append(line)
    svg = "\n".join(out)
    if mode == "construction":
        svg = svg.replace("</svg>", overlay + "</svg>")
    return svg


def overlay(cx, cy, squircle):
    g = ['<g opacity="0.5" stroke="#31456B" stroke-width="1" fill="none">']
    for x in range(0, 513, 32):
        g.append(f'<line x1="{x}" y1="0" x2="{x}" y2="512"/>')
        g.append(f'<line x1="0" y1="{x}" x2="512" y2="{x}"/>')
    for r in (70, 110, 150, 178):
        g.append(f'<circle cx="{cx}" cy="{cy}" r="{r}"/>')
    k = 178 * 0.70710678
    g.append(f'<line x1="{cx - k:.0f}" y1="{cy - k:.0f}" x2="{cx + k:.0f}" y2="{cy + k:.0f}"/>')
    g.append(f'<line x1="{cx + k:.0f}" y1="{cy - k:.0f}" x2="{cx - k:.0f}" y2="{cy + k:.0f}"/>')
    g.append(f'<line x1="{cx}" y1="10" x2="{cx}" y2="502" stroke-width="1.4"/>')
    g.append(f'<line x1="10" y1="{cy}" x2="502" y2="{cy}" stroke-width="1.4"/>')
    if squircle:
        g.append('<rect x="60" y="60" width="392" height="392" rx="115" stroke-width="1.6"/>')
    g.append("</g>")
    return "".join(g)


def build():
    for key, name, tag, master, (cx, cy), squircle in MARKS:
        for mode in ("color", "mono", "construction"):
            svg = derive(master, mode, overlay(cx, cy, squircle))
            with open(f"{OUT}/svg/{key}_{mode}.svg", "w") as f:
                f.write(svg)
            cairosvg.svg2png(bytestring=svg.encode(),
                             write_to=f"{OUT}/png/{key}_{mode}.png",
                             output_width=512, output_height=512)
        print("built", key)


def load_font(size, bold=True):
    p = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf" if bold else \
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
    return ImageFont.truetype(p, size)


def compose():
    W = 1870
    CELL, PAD, GUT = 520, 24, 108
    HEADER, FOOT = 150, 64
    ROWH = CELL + 2 * PAD
    H = HEADER + 3 * ROWH + 96 + FOOT
    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)
    f_title, f_sub = load_font(42), load_font(20, False)
    f_row, f_name = load_font(19), load_font(24)
    f_tag, f_foot = load_font(16, False), load_font(15, False)

    d.text((70, 42), "IUM  \u00b7  LOGOMARK SYSTEM", font=f_title, fill="#F2F6FF")
    d.text((70, 100), "Concept Set 03 \u2014 hand-drawn smooth geometry \u00b7 filleted forms \u00b7 "
                      "ribbon gradients \u00b7 three states: color / monochrome / construction",
           font=f_sub, fill="#7C8DB0")

    modes = ["color", "mono", "construction"]
    mode_labels = ["COLOR", "WHITE MONOCHROME", "CONSTRUCTION \u00b7 GRID & GOLDEN CIRCLES"]
    for r, mode in enumerate(modes):
        y0 = HEADER + r * ROWH
        d.text((70, y0 + PAD - 6), mode_labels[r], font=f_row, fill="#5D6E8C")
        d.line([(70, y0 + PAD + 18), (W - 70, y0 + PAD + 18)], fill="#141F33", width=1)
        for c, (key, name, tag, _, _, _) in enumerate(MARKS):
            x0 = GUT + 46 + c * (CELL + PAD)
            cell_x, cell_y = x0, y0 + PAD + 34
            png = Image.open(f"{OUT}/png/{key}_{mode}.png").convert("RGBA")
            mark = png.resize((460, 460), Image.LANCZOS)
            img.paste(mark, (cell_x + (CELL - 460) // 2, cell_y + (CELL - 460) // 2), mark)
            d.rectangle([cell_x, cell_y, cell_x + CELL, cell_y + CELL],
                        outline="#101B2E", width=1)
            if r == 0:
                cx = cell_x + CELL // 2
                t1 = f"{c+1:02d} \u00b7 {name}"
                d.text((cx - d.textlength(t1, font=f_name) / 2, HEADER + 3 * ROWH + 26),
                       t1, font=f_name, fill="#F2F6FF")
                d.text((cx - d.textlength(tag, font=f_tag) / 2, HEADER + 3 * ROWH + 58),
                       tag, font=f_tag, fill="#5D6E8C")
    d.text((70, H - 44), "Review only \u2014 nothing merged \u00b7 pick a mark \u00b7 "
                         "hand-written SVG sources: design/logo-variants/set03/ \u00b7 IUM upload engine",
           font=f_foot, fill="#44536F")
    out = f"{OUT}/showcase_set03.png"
    img.save(out)
    print("showcase:", out, img.size)


if __name__ == "__main__":
    build()
    compose()
