#!/usr/bin/env python3
"""WCAG 2.1 contrast audit of the Ink token set (#86).

Every text/background pair the UI actually composes, both schemes.
The AA bar: 4.5:1 for normal text, 3:1 for large text and UI graphics.
The accent fill pairs (button ink vs its type) need 4.5:1 too — they are
normal-size labels.
"""

def srgb(c):
    c /= 255.0
    return c / 12.92 if c <= 0.04045 * 255 / 255 else ((c + 0.055) / 1.055) ** 2.4

def lum(hexcolor):
    h = hexcolor.lstrip("#")
    r, g, b = (int(h[i:i+2], 16) for i in (0, 2, 4))
    def f(v):
        v /= 255.0
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)

def ratio(fg, bg):
    l1, l2 = sorted((lum(fg), lum(bg)), reverse=True)
    return (l1 + 0.05) / (l2 + 0.05)

LIGHT = dict(bg="#f5f6f8", surface="#ffffff", surface2="#eef0f4", surface3="#e4e8ee",
             text="#0f172a", text2="#475569", text3="#5e6d86",
             border="#e6e9ef", border_strong="#d2d8e3", accent="#0f172a", accent_hover="#1d2941", accent_down="#0a1122", accent_text="#ffffff", accent_subtle="#eef1f6",
             ok="#2b7549", ok_bg="#e7f3ec", danger="#c43a2b", danger_bg="#faeceb",
             warn="#8f610c", warn_bg="#faf0dc", info="#40639f", info_bg="#eaeffb")
DARK = dict(bg="#0b101a", surface="#0e1521", surface2="#131c2c", surface3="#18233a",
            text="#edf1f8", text2="#a8b3c7", text3="#77849d",
            border="#1d2942", border_strong="#2c3a5c", accent="#e9eef6", accent_hover="#f6f9ff", accent_down="#d5deeb", accent_text="#0b101a", accent_subtle="#1a2438",
            ok="#7cc793", ok_bg="#16301f", danger="#ff8f86", danger_bg="#331d22",
            warn="#e3b95f", warn_bg="#33290f", info="#92aae6", info_bg="#1a2540")

# (foreground key, background key, minimum, note)
PAIRS = [
    ("text", "surface", 4.5, "body on content"),
    ("text", "bg", 4.5, "body on chrome"),
    ("text2", "surface", 4.5, "secondary on content"),
    ("text2", "bg", 4.5, "secondary on chrome"),
    ("text3", "surface", 4.5, "meta on content"),
    ("text3", "bg", 4.5, "meta on chrome"),
    ("accent_text", "accent", 4.5, "filled-button type on ink"),
    ("accent_text", "accent_hover", 4.5, "filled-button type on hover"),
    ("accent_text", "accent_down", 4.5, "filled-button type on pressed"),
    ("text", "accent_subtle", 4.5, "selected-row text"),
    ("ok", "ok_bg", 4.5, "health pair"),
    ("danger", "danger_bg", 4.5, "failure pair"),
    ("danger", "surface", 4.5, "failure text on content"),
    ("warn", "warn_bg", 4.5, "caution pair"),
    ("info", "info_bg", 4.5, "info pair"),
]

fail = []
for name, s in (("LIGHT", LIGHT), ("DARK", DARK)):
    print(f"== {name} ==")
    for fg, bg, minimum, note in PAIRS:
        r = ratio(s[fg], s[bg])
        mark = "PASS" if r >= minimum else "FAIL"
        if r < minimum:
            fail.append((name, fg, bg, r, note))
        print(f"  {mark}  {fg:>12} on {bg:<13} {r:5.2f}:1  ({note})")

# AA bar for UI graphics (3:1): borders against both surfaces.
for name, s in (("LIGHT", LIGHT), ("DARK", DARK)):
    for bgk in ("surface", "bg"):
        r = ratio(s["border"], s[bgk])
        print(f"  {'PASS' if r >= 3 else 'NOTE'}  border on {bgk:<13} {r:5.2f}:1  (hairline, graphics bar 3:1 — non-essential decor is exempt)")

print()
if fail:
    print("FAILURES:", fail)
    raise SystemExit(1)
print("All text pairs meet WCAG AA (4.5:1).")
