/**
 * The quiet chrome gate (#77, re-homed by #88): the TOOLBAR is commands
 * only — it carries no brand block (the logo ladder's Chrome rung: none,
 * enforced here in the component, not only the brand sheet) and never
 * renders the engine's raw status sentence, a version string, or the
 * adapter roster. The engine's one ambient signal lives in the STATUS BAR
 * as dot + short word; the engine's own sentence stays inspectable in the
 * status bar's tooltip and Settings → About, never as permanent chrome.
 */
// @vitest-environment jsdom
import { describe, expect, it, vi, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";

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
    applyCredentials: () => Promise.resolve({ action: "replaced", lanes: [] }),
    probeCredentials: () =>
      Promise.resolve({ checked: false, ok: true, sentence: "", final: true }),
    secretMode: () => Promise.resolve("keyring"),
    telemetryStatus: () =>
      Promise.resolve({ consent: true, count: 0, path: "/tmp/telemetry.jsonl" }),
    telemetrySetConsent: () => Promise.resolve(),
    telemetryClear: () => Promise.resolve(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// no auto-cleanup under vitest (globals off) — the #87 lesson
afterEach(cleanup);

describe("the quiet chrome (#77 → #88)", () => {
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
    // the engine's own words stay inspectable — status-bar tooltip + About,
    // never as permanent chrome
    const bar = container.querySelector(".statusbar");
    expect(bar, "the status bar carries the one ambient signal").not.toBeNull();
    expect(bar!.querySelector(".engine-pill")?.getAttribute("title")).toContain(
      "ium-core",
    );
    expect(text).not.toMatch(/ium-core v/); // nowhere in the shell surface
  });

  it("the toolbar is commands only — no brand block, no logomark, no wordmark", async () => {
    const { default: App } = await import("./App");
    const { container } = render(<App />);
    const header = container.querySelector("header");
    expect(header).not.toBeNull();
    expect(header!.querySelector(".brand"), "no .brand in the working chrome").toBeNull();
    expect(header!.querySelector(".logomark"), "no logomark in the chrome").toBeNull();
    expect(header!.textContent).not.toContain("IUM —");
    expect(header!.textContent).not.toContain("Internet Upload Manager");
  });
});
