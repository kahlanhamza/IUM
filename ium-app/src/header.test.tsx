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
});
