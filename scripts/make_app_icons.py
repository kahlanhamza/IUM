#!/usr/bin/env python3
"""Generate the IUM app icon set (issue #14) — minimal, honest, no assets.

One geometric source drawing (arrow-up into a tray = upload), rendered by
PIL at 1024 px and exported to the sizes Tauri v2 expects next to
src-tauri/tauri.conf.json. Committed so codegen never depends on network
tooling. Usage: python3 scripts/make_app_icons.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path("/home/z/my-project/ium-app/src-tauri/icons")
SIZES = [32, 128, 256]  # 128@2x == 256

BG = (13, 27, 42, 255)      # deep navy
FG = (46, 196, 182, 255)    # teal arrow
TRAY = (224, 225, 221, 255) # off-white tray


def draw(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), BG)
    d = ImageDraw.Draw(img)
    s = size / 1024.0

    # rounded-rect tile
    r = int(180 * s)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=r, fill=BG)

    # tray: thick baseline with two upturned ends
    y = int(760 * s)
    lw = int(64 * s)
    x0, x1 = int(180 * s), int(844 * s)
    d.line([(x0, y), (x1, y)], fill=TRAY, width=lw)
    d.line([(x0, y), (x0, y - int(150 * s))], fill=TRAY, width=lw)
    d.line([(x1, y), (x1, y - int(150 * s))], fill=TRAY, width=lw)

    # upload arrow: shaft + head
    cx = size // 2
    shaft_top = int(250 * s)
    d.line([(cx, shaft_top + int(120 * s)), (cx, y - int(120 * s))], fill=FG, width=lw)
    head = [(cx - int(170 * s), shaft_top + int(240 * s)), (cx, shaft_top), (cx + int(170 * s), shaft_top + int(240 * s))]
    d.polygon(head, fill=FG)
    return img


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    src = draw(1024)
    src.save(ROOT / "icon.png")
    src.resize((512, 512), Image.LANCZOS).save(ROOT / "512x512.png")
    for sz in SIZES:
        src.resize((sz, sz), Image.LANCZOS).save(ROOT / f"{sz}x{sz}.png")
    src.resize((256, 256), Image.LANCZOS).save(ROOT / "128x128@2x.png")
    src.resize((128, 128), Image.LANCZOS).save(ROOT / "128x128.png")
    # .ico for Windows resource embedding
    src.resize((256, 256), Image.LANCZOS).save(
        ROOT / "icon.ico", format="ICO", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )
    print(f"icons written to {ROOT}")


if __name__ == "__main__":
    main()
