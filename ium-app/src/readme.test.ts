/**
 * The README gate (#77): the README ships with receipts — the committed
 * product screenshots are embedded, and the roadmap section reflects the
 * tree's real state (v0.2.0, product-bar lanes closed, the telemetry
 * stance on record), not a stale M0–M7 / v0.1.0 snapshot.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const README_PATH = fileURLToPath(new URL("../../README.md", import.meta.url));
const read = () => readFileSync(README_PATH, "utf8").replace(/\r\n/g, "\n");

const SHOTS = [
  "docs/screenshots/01-main-window.png",
  "docs/screenshots/02-create-upload-filled.png",
  "docs/screenshots/03-inspector-chunks.png",
  "docs/screenshots/04-settings-privacy.png",
  "docs/screenshots/05-settings-credentials.png",
  "docs/screenshots/06-main-window-light.png",
];

describe("the README ships with receipts (#77)", () => {
  it("embeds every committed product screenshot", () => {
    const md = read();
    const missing = SHOTS.filter((s) => !md.includes(s));
    expect(missing, "screenshots not embedded in README").toEqual([]);
  });

  it("the screenshots section exists before the architecture section", () => {
    const md = read();
    const shots = md.indexOf("## Screenshots");
    const arch = md.indexOf("## Architecture");
    expect(shots).toBeGreaterThan(-1);
    expect(arch).toBeGreaterThan(shots);
  });

  it("no stale roadmap claims — the tree has moved past v0.1.0 / M0–M7", () => {
    const md = read();
    expect(md).not.toContain("v0.1.0");
    expect(md).not.toContain("M0–M7");
  });

  it("the product's trust surfaces are stated — telemetry stance and honesty", () => {
    const md = read();
    expect(md).toMatch(/telemetry|Privacy/);
    expect(md).toMatch(/honest/i);
  });
});
