/**
 * The header gate (#77): the chrome is quiet — big-tech rule (HIG "defer to
 * content"). The header carries identity and one ambient status signal; it
 * NEVER renders the engine's raw status sentence, a version string, or the
 * adapter roster (those live in the tooltip and Settings → About). The brand
 * block carries the real logomark, not the generic upload stroke icon.
 */
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";

const ENGINE_SENTENCE =
  "ium-core v0.1.1 ready — adapters: http_one_shot, s3_multipart, tus";

vi.mock("./api", () => ({
  api: {
    engineStatus: () => Promise.resolve(ENGINE_SENTENCE),
    listAdapters: () => Promise.resolve(["http_oneshot", "s3_multipart", "tus"]),
    listJobs: () => Promise.resolve([]),
    snapshot: () =>
      Promise.resolve({
        job: {
          id: "x",
          source_path: "/x",
          source_size: 1,
          adapter: "tus",
          endpoint: "https://x",
          state: "done",
          capability_badge: "resumable",
          created_at: 1,
          updated_at: 2,
        },
        chunks: [],
        events: [],
      }),
    statFile: () => Promise.resolve(1048576),
    createJob: () => Promise.reject(new Error("unused")),
    inspectChunks: () => Promise.reject(new Error("unused")),
    pauseJob: () => Promise.resolve(),
    resumeSignal: () => Promise.resolve(),
    cancelJob: () => Promise.resolve(),
    credentialsSchema: () => Promise.resolve([]),
    credentialsGet: () => Promise.resolve({}),
    credentialsSet: () => Promise.resolve(),
    credentialsDelete: () => Promise.resolve(),
    telemetryStatus: () =>
      Promise.resolve({ consent: true, count: 0, path: "/tmp/telemetry.jsonl" }),
    telemetrySetConsent: () => Promise.resolve(),
    telemetryClear: () => Promise.resolve(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

describe("the quiet header (#77)", () => {
  it("never renders the engine's raw sentence, a version, or the adapter roster", async () => {
    const { default: App } = await import("./App");
    const { container, findAllByText } = render(<App />);
    await findAllByText("Connected"); // the ambient status is two words
    const text = container.textContent ?? "";
    const header = container.querySelector("header");
    expect(header).not.toBeNull();
    const headerText = header?.textContent ?? "";
    expect(headerText).not.toMatch(/v\d+\.\d+\.\d+/);
    expect(headerText).not.toContain("adapters:");
    expect(headerText).not.toContain("http_oneshot");
    // the engine's own words stay inspectable — tooltip + About, not chrome
    expect(header!.querySelector(".engine-pill")?.getAttribute("title")).toContain(
      "ium-core",
    );
    expect(text).not.toMatch(/ium-core v/); // nowhere in the shell surface
  });

  it("the brand block is the constructed logomark — real SVG paths, not the generic icon", async () => {
    const { default: App } = await import("./App");
    const { container } = render(<App />);
    const mark = container.querySelector("header .brand .logomark");
    expect(mark, "header renders a .logomark").not.toBeNull();
    expect(mark!.tagName.toLowerCase()).toBe("svg");
    // constructed geometry: more than one path, no raster fallbacks
    expect(mark!.querySelectorAll("path").length).toBeGreaterThanOrEqual(2);
    expect(mark!.querySelector("image")).toBeNull();
    expect(mark!.getAttribute("aria-hidden")).toBe("true");
  });

  it("the logomark is the dart (#79) — twin arcs, round caps, no tile, currentColor", async () => {
    const { default: App } = await import("./App");
    const { container } = render(<App />);
    const mark = container.querySelector("header .brand .logomark");
    expect(mark, "header renders a .logomark").not.toBeNull();
    // the dart is exactly two ribbons — the main arc and its echo
    const paths = mark!.querySelectorAll("path");
    expect(paths.length, "the dart is two arcs").toBe(2);
    for (const p of Array.from(paths)) {
      // smooth geometry only: arcs, round caps — the owner's construction ruling
      expect(p.getAttribute("d")).toMatch(/^M\s/);
      expect(p.getAttribute("stroke-linecap")).toBe("round");
      expect(p.getAttribute("stroke-width")).toBeTruthy();
      // flat ink discipline: the mark paints with currentColor, no hardcoded
      // brand hex, no fills, no blends
      expect(p.getAttribute("stroke")).toBe("currentColor");
      expect(p.getAttribute("fill")).toBe("none");
    }
    // the teal tile is gone — no rects in the mark, ever
    expect(mark!.querySelector("rect")).toBeNull();
    // and no gradient machinery may ride along (the owner banned it outright)
    expect(mark!.innerHTML).not.toMatch(/gradient/i);
    expect(mark!.querySelector("defs")).toBeNull();
  });
});
