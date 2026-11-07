#!/usr/bin/env python3
"""Drive the real IUM frontend (vite dev + Tauri IPC mock) and capture
product screenshots into download/screenshots/. Dark pass + one light pass.
"""

import os

from playwright.sync_api import sync_playwright

BASE = "http://localhost:5173"
OUT = "/home/z/my-project/download/screenshots"
os.makedirs(OUT, exist_ok=True)


def main() -> None:
    with sync_playwright() as p:
        browser = p.chromium.launch(
            args=["--no-sandbox", "--disable-dev-shm-usage"]
        )

        # ---------------------------------------------------- dark pass
        ctx = browser.new_context(
            viewport={"width": 1440, "height": 900},
            device_scale_factor=2,
            color_scheme="dark",
        )
        pg = ctx.new_page()
        pg.goto(BASE)
        pg.wait_for_selector("table tbody tr", timeout=20000)
        pg.wait_for_timeout(3300)  # 3+ live samples -> speed renders
        pg.screenshot(path=f"{OUT}/01-main-window.png")

        # filled intake: path stat'ed, endpoint typed, protocol picked
        pg.fill("#upload-path", "/home/you/datasets/model-weights-14b.safetensors")
        pg.select_option("#upload-adapter", "mega")
        pg.fill("#upload-endpoint", "https://mega.nz/fm/ium-weights")
        pg.wait_for_timeout(350)
        pg.screenshot(path=f"{OUT}/02-create-upload-filled.png")

        # inspector drawer on the live job
        pg.click("table tbody tr:nth-child(1)")
        pg.wait_for_selector('div[role="dialog"]', timeout=8000)
        pg.wait_for_timeout(450)
        pg.screenshot(path=f"{OUT}/03-inspector-chunks.png")
        pg.keyboard.press("Escape")
        pg.wait_for_timeout(250)

        # settings: privacy card leads the dialog
        pg.click("#app-settings")
        pg.wait_for_selector('div[aria-label^="Settings"]', timeout=8000)
        pg.wait_for_timeout(600)
        pg.screenshot(path=f"{OUT}/04-settings-privacy.png")

        # scrolled into the credentials vault sections
        pg.eval_on_selector(
            'div[aria-label^="Settings"]',
            "el => el.scrollTo(0, 1150)",
        )
        pg.wait_for_timeout(350)
        pg.screenshot(path=f"{OUT}/05-settings-credentials.png")
        ctx.close()

        # ---------------------------------------------------- light pass
        ctx2 = browser.new_context(
            viewport={"width": 1440, "height": 900},
            device_scale_factor=2,
            color_scheme="light",
        )
        pg2 = ctx2.new_page()
        pg2.goto(BASE)
        pg2.wait_for_selector("table tbody tr", timeout=20000)
        pg2.wait_for_timeout(3300)
        pg2.screenshot(path=f"{OUT}/06-main-window-light.png")
        ctx2.close()

        browser.close()


if __name__ == "__main__":
    main()
