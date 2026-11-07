#!/usr/bin/env python3
"""Set04 — the dart colorways, FLAT.

The owner approved the set03 dart geometry ("I like 02 dart") and banned
gradients outright ("the gradient was just the reference image"). So this
script writes no geometry of its own: the two smooth concentric-arc ribbons
are copied byte-for-byte from set03, and only the stroke inks change —
solid flat colors, no <defs>, no gradient stops, nothing but solid stroke
attributes in the SVG.

Colorways:
  flat_cobalt     both ribbons #2563EB        — one ink, infrastructure-grade
  flat_duo_cyan   outer #22D3EE, inner #2563EB — two flat tones, keeps the energy
  flat_duo_violet outer #2563EB, inner #7C3AED  — two flat tones, keeps the violet echo
  flat_ink        both #0F172A                 — stealth option for light surfaces
  mono_white      both #FFFFFF                 — dark surfaces / favicon fallback
"""
import os
import shutil

import cairosvg
from PIL import Image, ImageDraw, ImageFont

BASE = "/home/z/my-project/download/logo_set04"
SVG_DIR = os.path.join(BASE, "svg")
PNG_DIR = os.path.join(BASE, "png")
os.makedirs(SVG_DIR, exist_ok=True)
os.makedirs(PNG_DIR, exist_ok=True)

# --- frozen geometry, verbatim from set03 svg/02_dart_color.svg -------------
# inner echo ribbon: shorter arc, thinner stroke; main ribbon: longer arc,
# thicker stroke; both round-capped. Only {INK_IN}/{INK_OUT} change.
TPL = """<svg width="512" height="512" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
  <!-- IUM dart — set03 geometry frozen, flat solid inks (owner ruling: no color blends) -->
  <g transform="translate(11 -10.8) scale(1.15)">
    <path fill="none" stroke="{INK_IN}" stroke-width="46" stroke-linecap="round"
      d="M 214 314.4 A 104 104 0 0 1 261.9 227.4"/>
    <path fill="none" stroke="{INK_OUT}" stroke-width="54" stroke-linecap="round"
      d="M 148 314 A 170 170 0 0 1 278 150"/>
  </g>
</svg>
"""

COLORWAYS = [
    ("flat_cobalt",     "#2563EB", "#2563EB"),
    ("flat_duo_cyan",   "#2563EB", "#22D3EE"),
    ("flat_duo_violet", "#7C3AED", "#2563EB"),
    ("flat_ink",        "#0F172A", "#0F172A"),
    ("mono_white",      "#FFFFFF", "#FFFFFF"),
]

# --- 1. hand-write the SVGs (no defs, no gradient tags — grep-verifiable) ---
for name, ink_in, ink_out in COLORWAYS:
    svg = TPL.format(INK_IN=ink_in, INK_OUT=ink_out)
    with open(os.path.join(SVG_DIR, f"{name}.svg"), "w") as f:
        f.write(svg)

# hard proof for the owner: the word "gradient" must not appear anywhere
for f in sorted(os.listdir(SVG_DIR)):
    src = open(os.path.join(SVG_DIR, f)).read()
    assert "gradient" not in src.lower() and "<defs>" not in src, f
print("svg written: 5 files, 0 gradients, 0 defs")

# --- 2. render PNGs at the QA ladder ----------------------------------------
SIZES = [512, 96, 48, 24]
for name, *_ in COLORWAYS:
    src = os.path.join(SVG_DIR, f"{name}.svg")
    for size in SIZES:
        cairosvg.svg2png(url=src, write_to=os.path.join(PNG_DIR, f"{name}_{size}.png"),
                         output_width=size, output_height=size)
print("png rendered:", SIZES)


def load_font(bold=False, size=28):
    import glob
    pats = ["**/NotoSansSC-Bold*", "**/NotoSansSC-SemiBold*"] if bold else ["**/NotoSansSC-Regular*"]
    for p in pats:
        hits = glob.glob(f"/usr/share/fonts/{p}", recursive=True)
        if hits:
            return ImageFont.truetype(hits[0], size)
    return ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", size)


def rounded(draw, box, r, fill):
    draw.rounded_rectangle(box, radius=r, fill=fill)


# --- 3. the review sheet ------------------------------------------------------
# Layout: title; row A = 4 flat colorways on white cards (light surfaces);
# row B = same 4 on navy cards (dark surfaces); row C = white mono on navy +
# ink mono on white at large size; row D = cobalt + duo_cyan size ladder
# 96/48/24 on navy. Flat ink must earn its keep on both surface families.
W, H = 2040, 1660
sheet = Image.new("RGB", (W, H), "#F6F8FB")
d = ImageDraw.Draw(sheet)
NAVY = (11, 18, 32)

t_font = load_font(True, 44)
s_font = load_font(True, 26)
l_font = load_font(False, 22)

d.text((60, 40), "IUM logomark — set04 — the dart, FLAT (no gradients, owner ruling)", font=t_font, fill=(15, 23, 42))
d.text((60, 96), "set03 geometry frozen · solid inks only · pick a colorway", font=l_font, fill=(100, 116, 139))

names = ["flat_cobalt", "flat_duo_cyan", "flat_duo_violet", "flat_ink"]
labels = ["cobalt — one ink", "duo cyan", "duo violet", "ink — stealth"]

def card(x, y, dark, png_name, label, size=300):
    bg = NAVY if dark else (255, 255, 255)
    rounded(d, (x, y, x + 380, y + 420), 24, bg)
    img = Image.open(os.path.join(PNG_DIR, f"{png_name}_{512}.png")).resize((size, size), Image.LANCZOS)
    sheet.paste(img, (x + 40, y + 30), img)
    d.text((x + 40, y + 360), label, font=s_font, fill=(255, 255, 255) if dark else (30, 41, 59))

# row A — light cards
xA = 60
for i, (n, lab) in enumerate(zip(names, labels)):
    card(xA + i * 490, 170, False, n, lab)
# row B — dark cards
for i, (n, lab) in enumerate(zip(names, labels)):
    card(xA + i * 490, 655, True, n, lab)

d.text((60, 614), "DARK SURFACES — same four inks on navy:", font=s_font, fill=(71, 85, 105))

# row C — mono story: white mono on navy, ink mono on white
card(60, 1110, True, "mono_white", "mono — white on dark")
card(550, 1110, False, "flat_ink", "mono — ink on white")

# row D — size ladder for the two leading candidates on navy
d.text((1040, 1080), "SIZE LADDER — cobalt (top) vs duo cyan (bottom) at 96/48/24:", font=s_font, fill=(71, 85, 105))
for row, (nm, y0) in enumerate([("flat_cobalt", 1110), ("flat_duo_cyan", 1330)]):
    for col, s in enumerate([96, 48, 24]):
        x = 1040 + col * 330
        rounded(d, (x, y0, x + 300, y0 + 190), 16, NAVY)
        img = Image.open(os.path.join(PNG_DIR, f"{nm}_{s}.png"))
        img = img.resize((s * 2, s * 2), Image.NEAREST if s <= 48 else Image.LANCZOS)
        sheet.paste(img, (x + 150 - s, y0 + 95 - s), img)
        d.text((x + 20, y0 + 150), f"{s}px", font=l_font, fill=(148, 163, 184))

out = os.path.join(BASE, "showcase_set04_dart_flat.png")
sheet.save(out)
shutil.copy(out, os.path.join(PNG_DIR, "showcase_set04_dart_flat.png"))
print("showcase:", out)

# --- 4. sanity: every PNG exists and is non-trivial --------------------------
for f in sorted(os.listdir(PNG_DIR)):
    p = os.path.join(PNG_DIR, f)
    im = Image.open(p).convert("RGBA")
    px = im.getdata()
    ink = sum(1 for r, g, b, a in px if a > 10)
    assert ink > len(list(px)) * 0.005, f"{f} looks blank"
print("all PNGs verified non-blank")
