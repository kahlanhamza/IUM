#!/usr/bin/env python3
"""Screenshot the IUM mockups at 2x for crisp review."""
from playwright.sync_api import sync_playwright

BASE = "/home/z/my-project/design/mockup"
DL = "/home/z/my-project/download"
SHOTS = [
    ("light.html", "ium-ui-mock-1-light.png"),
    ("dark.html", "ium-ui-mock-2-dark-inspector.png"),
    ("settings.html", "ium-ui-mock-3-settings.png"),
]

with sync_playwright() as p:
    b = p.chromium.launch()
    ctx = b.new_context(viewport={"width": 1360, "height": 880}, device_scale_factor=2)
    page = ctx.new_page()
    for src, out in SHOTS:
        page.goto(f"file://{BASE}/{src}")
        page.wait_for_timeout(350)
        page.screenshot(path=f"{DL}/{out}")
        print("shot", out)
    b.close()
