#!/usr/bin/env python3
"""Concept Set 02 — futuristic geometric logomarks for IUM.

Three marks in the owner's reference language (folded gradient planes,
rounded terminals, dark negative space), each rendered in three states:
color / white monochrome / construction grid. Composes a dark showcase board.

Marks:
  01 Vault  — an isometric crate, opened to the sky (the folded ribbon box)
  02 Dart   — two layered ascending arrow planes (the fast, layered ascent)
  03 Aegis  — a folded hex ribbon with an arrow rising through it
"""
import math
import os

import cairosvg
from PIL import Image, ImageDraw, ImageFont

BASE = "/home/z/my-project/download/ium_logo_variants"
OUT = os.path.join(BASE, "set02")
os.makedirs(os.path.join(OUT, "svg"), exist_ok=True)
os.makedirs(os.path.join(OUT, "png"), exist_ok=True)

BG = "#05070D"
INK = "#9AA7BA"          # construction tone
GAP = "#05070D"          # facet separation on dark

N = 0.70710678


# ---------------------------------------------------------------- helpers
def rounded(path_primitives, w):
    """stroke-with-own-paint trick -> rounded corners on filled polygons."""
    return w


def pts_to_path(pts, close=True):
    d = "M " + " L ".join(f"{x:.1f} {y:.1f}" for x, y in pts)
    return d + (" Z" if close else "")


def lin_grad(gid, x1, y1, x2, y2, c1, c2):
    return (f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" '
            f'x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">'
            f'<stop offset="0%" stop-color="{c1}"/>'
            f'<stop offset="100%" stop-color="{c2}"/></linearGradient>')


def resolve(paint):
    """bare gradient name -> url(#name); pass through colors/none."""
    if paint and paint not in ("none", "DARK") and not paint.startswith(("#", "url")):
        return f"url(#{paint})"
    return paint


def render_prims(prims, mode, defs=""):
    """prims: list of dicts {d, fill, stroke, w, cap, join}.
    mode: color | mono | construction"""
    body = []
    for p in prims:
        d = p["d"]
        if mode == "color":
            fill, stroke, w = resolve(p.get("fill", "none")), resolve(p.get("stroke", "none")), p.get("w", 0)
            if p.get("keep_dark"):
                fill, stroke, w = GAP, GAP, p.get("w", 12)
        elif mode == "mono":
            fill = "#FFFFFF" if p.get("fill", "none") != "none" else "none"
            stroke = "#FFFFFF" if p.get("stroke", "none") != "none" else GAP
            if fill != "none":
                w = max(10, int(p.get("w", 10) * 0.5))
            else:
                w = p.get("w", 10)
            if p.get("keep_dark"):
                fill, stroke, w = GAP, GAP, 8
        else:  # construction
            fill = INK if p.get("fill", "none") != "none" else "none"
            stroke = INK if p.get("stroke", "none") != "none" else GAP
            w = max(6, int(p.get("w", 8) * 0.5))
            if p.get("keep_dark"):
                fill, stroke, w = GAP, GAP, 6
        cap, join = p.get("cap", "round"), p.get("join", "round")
        body.append(f'<path d="{d}" fill="{fill}" stroke="{stroke}" '
                    f'stroke-width="{w}" stroke-linecap="{cap}" stroke-linejoin="{join}"/>')
    return (f'<svg width="512" height="512" viewBox="0 0 512 512" '
            f'xmlns="http://www.w3.org/2000/svg"><defs>{defs}</defs>'
            f'<rect width="512" height="512" fill="none"/>' + "".join(body) + "</svg>")


def construction_overlay(cx, cy, squircle=False):
    g = [f'<g opacity="0.55" stroke="#31456B" stroke-width="1" fill="none">']
    for x in range(0, 513, 32):
        g.append(f'<line x1="{x}" y1="0" x2="{x}" y2="512"/>')
        g.append(f'<line x1="0" y1="{x}" x2="512" y2="{x}"/>')
    for r in (70, 110, 150, 178):
        g.append(f'<circle cx="{cx}" cy="{cy}" r="{r}"/>')
    k = 178 * N
    g.append(f'<line x1="{cx - k}" y1="{cy - k}" x2="{cx + k}" y2="{cy + k}"/>')
    g.append(f'<line x1="{cx + k}" y1="{cy - k}" x2="{cx - k}" y2="{cy + k}"/>')
    g.append(f'<line x1="{cx}" y1="10" x2="{cx}" y2="502" stroke-width="1.4"/>')
    g.append(f'<line x1="10" y1="{cy}" x2="502" y2="{cy}" stroke-width="1.4"/>')
    if squircle:
        g.append('<rect x="60" y="60" width="392" height="392" rx="115" stroke-width="1.6"/>')
    g.append("</g>")
    return "".join(g)


# ---------------------------------------------------------------- mark 1 · Vault
def vault_prims():
    dy = 7
    C = (256, 190 + dy)
    Np = (256, 120 + dy); E = (377.2, 190 + dy); S = (256, 260 + dy); W = (134.8, 190 + dy)
    d = 118
    Sw = (256, 260 + dy + d); Ww = (134.8, 190 + dy + d)
    Se = (377.2, 190 + dy + d)
    # opening diamond (scale 0.55 about top-face center)
    Tc = C
    open_pts = [tuple(Tc[i] + 0.55 * (p[i] - Tc[i]) for i in (0, 1)) for p in (Np, E, S, W)]
    prims = [
        {"d": pts_to_path([W, S, Sw, Ww]), "fill": "gLeft", "w": 18},
        {"d": pts_to_path([S, E, Se, Sw]), "fill": "gRight", "w": 18},
        {"d": pts_to_path([Np, E, S, W]), "fill": "gTop", "w": 18},
        {"d": pts_to_path(open_pts), "fill": "DARK", "w": 16, "keep_dark": True},
    ]
    return prims, (256, 256)


def vault_defs():
    dy = 7
    return (lin_grad("gTop", 200, 120 + dy, 360, 250 + dy, "#7DD3FC", "#3B82F6")
            + lin_grad("gLeft", 134.8, 190 + dy, 256, 378 + dy, "#1D4ED8", "#1E3A8A")
            + lin_grad("gRight", 256, 378 + dy, 377.2, 190 + dy, "#3B82F6", "#6D28D9"))


# ---------------------------------------------------------------- mark 2 · Dart
def dart_prims():
    dx, dy = -18, 18
    B = (196 + dx, 316 + dy)
    u = (N, -N); n = (N, N)
    H = tuple(B[i] + u[i] * 150 for i in (0, 1))
    T = tuple(B[i] + u[i] * 265.4 for i in (0, 1))
    # proper 7-point arrow: shaft corners + head-base notches + head corners + tip
    big = [tuple(B[i] - n[i] * 45 for i in (0, 1)),          # shaft bottom-left
           tuple(H[i] - n[i] * 45 for i in (0, 1)),          # head base left
           tuple(H[i] - n[i] * 92 for i in (0, 1)),          # head left corner
           T,                                                 # tip
           tuple(H[i] + n[i] * 92 for i in (0, 1)),          # head right corner
           tuple(H[i] + n[i] * 45 for i in (0, 1)),          # head base right
           tuple(B[i] + n[i] * 45 for i in (0, 1))]          # shaft bottom-right
    m = (-N, N)
    Bs = tuple(B[i] + m[i] * 96 + u[i] * 24 for i in (0, 1))
    small = [tuple(Bs[i] + 0.6 * (p[i] - B[i]) for i in (0, 1)) for p in big]
    prims = [
        {"d": pts_to_path(small), "fill": "gLow", "w": 18},
        {"d": pts_to_path(big), "fill": "gHi", "w": 20},
    ]
    return prims, (256, 250)


def dart_defs():
    return (lin_grad("gHi", 170, 320, 380, 120, "#22D3EE", "#2563EB")
            + lin_grad("gLow", 110, 400, 260, 250, "#7C3AED", "#4F46E5"))


# ---------------------------------------------------------------- mark 3 · Aegis
def aegis_prims():
    cx, cy, R = 256, 268, 150
    V = [(cx + R * math.cos(math.radians(a)), cy + R * math.sin(math.radians(a)))
         for a in (-90, -30, 30, 90, 150, 210)]
    hex_path = pts_to_path(V, close=False)  # stroke path (closed visually by round join)
    hex_path = pts_to_path(V, close=True)
    fold = pts_to_path([V[2], V[3], V[4]], close=False)
    arrow = pts_to_path([(191, 232), (256, 150), (321, 232), (274, 232),
                         (274, 345), (238, 345), (238, 232)])
    prims = [
        {"d": hex_path, "stroke": "gHex", "w": 46},
        {"d": fold, "stroke": "#4338CA", "w": 46},
        {"d": arrow, "fill": "gArr", "w": 16},
    ]
    return prims, (256, 268)


def aegis_defs():
    return (lin_grad("gHex", 140, 150, 370, 400, "#60A5FA", "#7C3AED")
            + lin_grad("gArr", 256, 150, 256, 345, "#67E8F9", "#2563EB"))


# ---------------------------------------------------------------- build
MARKS = [
    ("01_vault", "VAULT", "the folded crate, opened to the sky", vault_prims, vault_defs),
    ("02_dart", "DART", "layered planes, the fast ascent", dart_prims, dart_defs),
    ("03_aegis", "AEGIS", "the folded hex ribbon, an arrow through", aegis_prims, aegis_defs),
]


def build():
    for key, name, tag, prims_fn, defs_fn in MARKS:
        prims, (cx, cy) = prims_fn()
        # resolve named gradients for color mode
        defs = defs_fn().replace('id="', 'id="')
        color = render_prims(prims, "color", defs)
        mono = render_prims(prims, "mono")
        cons = (render_prims(prims, "construction")
                .replace("</svg>", construction_overlay(cx, cy) + "</svg>"))
        for mode, svg in (("color", color), ("mono", mono), ("construction", cons)):
            with open(f"{OUT}/svg/{key}_{mode}.svg", "w") as f:
                f.write(svg)
            cairosvg.svg2png(bytestring=svg.encode(),
                             write_to=f"{OUT}/png/{key}_{mode}.png",
                             output_width=512, output_height=512)
        print("built", key)


def load_font(size, bold=True):
    import glob
    pats = ["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"] if bold else \
           ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]
    for p in pats:
        if os.path.exists(p):
            return ImageFont.truetype(p, size)
    g = glob.glob("/usr/share/fonts/**/NotoSansSC-Regular*", recursive=True)
    return ImageFont.truetype(g[0], size) if g else ImageFont.load_default()


def compose():
    W = 1870
    CELL, PAD, GUT = 520, 24, 108
    HEADER, FOOT = 150, 64
    ROWH = CELL + 2 * PAD
    H = HEADER + 3 * ROWH + 96 + FOOT
    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)

    f_title = load_font(42)
    f_sub = load_font(20, bold=False)
    f_row = load_font(19)
    f_name = load_font(24)
    f_tag = load_font(16, bold=False)
    f_foot = load_font(15, bold=False)

    d.text((70, 42), "IUM  \u00b7  LOGOMARK SYSTEM", font=f_title, fill="#F2F6FF")
    d.text((70, 100), "Concept Set 02 \u2014 futuristic geometric \u00b7 folded gradient planes "
                      "\u00b7 three states: color / monochrome / construction",
           font=f_sub, fill="#7C8DB0")

    modes = ["color", "mono", "construction"]
    mode_labels = ["COLOR", "WHITE MONOCHROME", "CONSTRUCTION \u00b7 GRID & GOLDEN CIRCLES"]

    for r, mode in enumerate(modes):
        y0 = HEADER + r * ROWH
        d.text((70, y0 + PAD - 6), mode_labels[r], font=f_row, fill="#5D6E8C")
        d.line([(70, y0 + PAD + 18), (W - 70, y0 + PAD + 18)], fill="#141F33", width=1)
        for c, (key, name, tag, _, _) in enumerate(MARKS):
            x0 = GUT + 46 + c * (CELL + PAD)
            cell_x, cell_y = x0, y0 + PAD + 34
            png = Image.open(f"{OUT}/png/{key}_{mode}.png").convert("RGBA")
            mark = png.resize((440, 440), Image.LANCZOS)
            img.paste(mark, (cell_x + (CELL - 440) // 2, cell_y + (CELL - 440) // 2), mark)
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
                         "SVG sources: design/logo-variants/set02/ \u00b7 IUM upload engine",
           font=f_foot, fill="#44536F")
    out = f"{OUT}/showcase_set02.png"
    img.save(out)
    print("showcase:", out, img.size)


if __name__ == "__main__":
    build()
    compose()
