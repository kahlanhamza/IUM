/**
 * The launch window contract (#87): what tauri.conf.json must declare.
 *
 * Read in the NODE environment on purpose — the file is read off disk with
 * real path semantics, exactly the way the brand gate reads the sheet.
 *
 * The creation ORDER is the design: the main window is declared FIRST, so
 * WebDriver's handle 0 (the e2e lane's attach point, tauri-driver's
 * documented default) is the working chrome — never the splash. The splash
 * is declared second: small, frameless, centered, non-resizable, on top,
 * routed to #/launch, hiding nothing (main starts hidden anyway and the
 * splash floats over it until the handoff).
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const CONF_PATH = fileURLToPath(
  new URL("../src-tauri/tauri.conf.json", import.meta.url),
);

function windows(): Array<Record<string, unknown>> {
  const conf = JSON.parse(readFileSync(CONF_PATH, "utf8"));
  return conf.app.windows;
}

describe("the window contract (#87)", () => {
  it("declares main first and hidden (handle 0 stays the e2e seam)", () => {
    const wins = windows();
    expect(wins.length).toBeGreaterThanOrEqual(2);
    expect(wins[0].label).toBe("main");
    expect(wins[0].visible).toBe(false);
  });

  it("declares the splash: frameless, centered, on top, non-resizable, routed", () => {
    const splash = windows()[1];
    expect(splash.label).toBe("splash");
    expect(String(splash.url)).toContain("#/launch");
    expect(splash.decorations).toBe(false);
    expect(splash.center).toBe(true);
    expect(splash.resizable).toBe(false);
    expect(splash.alwaysOnTop).toBe(true);
    expect(splash.skipTaskbar).toBe(true);
  });
});
