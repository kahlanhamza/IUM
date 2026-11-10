#!/usr/bin/env python3
"""Render IUM logo concept variants to PNG and build a review contact sheet."""
import glob
import os

import cairosvg
from PIL import Image, ImageDraw, ImageFont

BASE = "/home/z/my-project/download/ium_logo_variants"
SVG_DIR = os.path.join(BASE, "svg")
PNG_DIR = os.path.join(BASE, "png")
os.makedirs(PNG_DIR, exist_ok=True)

VARIANTS = [
    ("01_beacon", "Beacon",
     "A ring with one break at the top; the arrow leaves through it. Quiet, one-color, "
     "infrastructure-grade minimal (Vercel / Linear school)."),
    ("02_strata", "Strata",
     "The file stack: the blue bar is the file leaving the queue. Calm, honest, "
     "light-surface craft; the only concept that tells the queue story."),
    ("03_surge", "Surge",
     "Double chevron on a blue gradient circle. Pure momentum and elevation; "
     "the strongest, fastest read at 16 px."),
    ("04_monogram", "Monogram",
     "Custom IU ligature: solid blue I with a single 45\u00b0 chamfer cut, stroked ink U "
     "on the same baseline. A signature mark, Stripe / Linear school."),
    ("05_cadence", "Cadence",
     "Ascending rounded bars with a chevron peak \u2014 throughput as a shape. "
     "\u201cStep by step, upward\u201d rendered literally."),
    ("06_zephyr", "Zephyr",
     "Two bars and a chevron on a 45\u00b0 runway. The only diagonal in the set \u2014 "
     "launch energy on a warm orange gradient."),
]


def load_fonts():
    cands_bold = glob.glob("/usr/share/fonts/**/NotoSansSC-Bold*", recursive=True) + \
                 glob.glob("/usr/share/fonts/**/NotoSansSC*SemiBold*", recursive=True)
    cands_reg = glob.glob("/usr/share/fonts/**/NotoSansSC-Regular*", recursive=True)
    fb = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
    fr = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
    bold = cands_bold[0] if cands_bold else fb
    reg = cands_reg[0] if cands_reg else fr
    return bold, reg


def wrap(draw, text, font, max_w):
    words, lines, cur = text.split(), [], ""
    for w in words:
        t = (cur + " " + w).strip()
        if draw.textlength(t, font=font) <= max_w:
            cur = t
        else:
            if cur:
                lines.append(cur)
            cur = w
    if cur:
        lines.append(cur)
    return lines


def main():
    bold_path, reg_path = load_fonts()
    f_title = ImageFont.truetype(bold_path, 46)
    f_sub = ImageFont.truetype(reg_path, 22)
    f_name = ImageFont.truetype(bold_path, 30)
    f_body = ImageFont.truetype(reg_path, 21)
    f_tiny = ImageFont.truetype(reg_path, 15)

    # 1) render 512 exports + verify non-blank
    for key, name, _ in VARIANTS:
        out = os.path.join(PNG_DIR, f"{key}_512.png")
        cairosvg.svg2png(url=os.path.join(SVG_DIR, f"{key}.svg"),
                         write_to=out, output_width=512, output_height=512)
        im = Image.open(out).convert("RGBA")
        lo, hi = im.convert("L").getextrema()
        assert lo != hi, f"{key} looks blank"
        print(f"rendered {key}_512.png  {im.size}  lum={lo}..{hi}")

    # 2) contact sheet
    W, HEADER_H, ROW_H = 1560, 140, 310
    H = HEADER_H + ROW_H * len(VARIANTS)
    sheet = Image.new("RGBA", (W, H), "#FFFFFF")
    d = ImageDraw.Draw(sheet)

    d.text((48, 34), "IUM \u00b7 Logomark Concepts", font=f_title, fill="#0F172A")
    d.text((48, 96), "Six directions \u00b7 shown at 256 / 96 / 48 / 24 px \u00b7 review only \u2014 nothing committed",
           font=f_sub, fill="#64748B")

    for i, (key, name, blurb) in enumerate(VARIANTS):
        y0 = HEADER_H + i * ROW_H
        cy = y0 + ROW_H // 2
        src = Image.open(os.path.join(PNG_DIR, f"{key}_512.png")).convert("RGBA")

        # main 256 preview
        m = src.resize((256, 256), Image.LANCZOS)
        sheet.alpha_composite(m, (48, cy - 128))
        # size ladder
        ladder = [(96, 400), (48, 540), (24, 632)]
        for size, x in ladder:
            s = src.resize((size, size), Image.LANCZOS)
            sheet.alpha_composite(s, (x, cy - size // 2))
        # 24px on dark chip for contrast check
        chip = Image.new("RGBA", (44, 44), (0, 0, 0, 0))
        cdraw = ImageDraw.Draw(chip)
        cdraw.rounded_rectangle([0, 0, 43, 43], radius=9, fill="#0B0D12")
        s24 = src.resize((24, 24), Image.LANCZOS)
        chip.alpha_composite(s24, (10, 10))
        sheet.alpha_composite(chip, (676, cy - 22))

        d.text((400, cy + 58), "96        48       24      24 on dark", font=f_tiny, fill="#94A3B8")

        # name + rationale
        tx = 780
        d.text((tx, cy - 74), f"{i+1:02d} \u00b7 {name}", font=f_name, fill="#0F172A")
        lines = wrap(d, blurb, f_body, W - tx - 60)
        ty = cy - 26
        for ln in lines[:4]:
            d.text((tx, ty), ln, font=f_body, fill="#475569")
            ty += 30

        if i:
            d.line([(40, y0), (W - 40, y0)], fill="#EEF1F4", width=2)

    out_sheet = os.path.join(BASE, "contact_sheet.png")
    sheet.convert("RGB").save(out_sheet, "PNG")
    print(f"sheet: {out_sheet}  {sheet.size}")


if __name__ == "__main__":
    main()
