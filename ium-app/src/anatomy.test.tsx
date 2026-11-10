/**
 * The anatomy gate (#88): the Ink chrome is not a look, it is a STRUCTURE —
 * the four-pane contract (command / navigate / work / explain) at the
 * mockup's exact metrics, enforced from the REAL stylesheet and the REAL
 * shell render. The brand sheet pins the ladder on paper; this file pins
 * the component: the working chrome carries no brand block, the sidebar
 * filters, the status bar speaks the short truth, and the shadow law
 * (hairlines carry edges; shadows only under floating surfaces) holds.
 */
// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi, afterEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/react";

// jsdom's import.meta.url is not a file: URL (the #87 lesson) — resolve
// from the process cwd, which vitest pins to the app root.
const CSS = readFileSync(resolve(process.cwd(), "src/theme.css"), "utf8").replace(/\r\n/g, "\n");

/** Pull a top-level rule body for a selector, e.g. block(".toolbar") -> CSS text. */
function block(selector: string): string {
  const i = CSS.indexOf(`${selector} {`);
  expect(i, `${selector} exists in theme.css`).toBeGreaterThanOrEqual(0);
  const end = CSS.indexOf("}", i);
  return CSS.slice(i, end);
}

/** The last selector line before each line — enough context for the law pins. */
function* ruleLines(): Generator<[string, string]> {
  let ctx = "";
  for (const line of CSS.split("\n")) {
    const t = line.trim();
    if (t.endsWith("{") || t.endsWith(",")) ctx = t;
    yield [ctx, line];
  }
}

const ENGINE_SENTENCE =
  "ium-core v0.2.1 ready — adapters: http_one_shot, s3_multipart, tus";

vi.mock("./api", () => ({
  api: {
    engineStatus: () => Promise.resolve(ENGINE_SENTENCE),
    listAdapters: () => Promise.resolve(["http_oneshot", "s3_multipart", "tus"]),
    listJobs: () =>
      Promise.resolve([
        {
          id: "a",
          source_path: "/data/alpha.bin",
          source_size: 16 * 1024 * 1024,
          adapter: "tus",
          endpoint: "https://x",
          state: "running",
          capability_badge: "resumable",
          created_at: 1,
          updated_at: 2,
        },
        {
          id: "b",
          source_path: "/data/beta.bin",
          source_size: 8 * 1024 * 1024,
          adapter: "tus",
          endpoint: "https://x",
          state: "paused",
          capability_badge: "resumable",
          created_at: 1,
          updated_at: 2,
        },
        {
          id: "c",
          source_path: "/data/gamma.bin",
          source_size: 4 * 1024 * 1024,
          adapter: "tus",
          endpoint: "https://x",
          state: "done",
          capability_badge: "resumable",
          created_at: 1,
          updated_at: 2,
        },
      ]),
    snapshot: () => Promise.reject(new Error("unused")),
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

// jsdom has no auto-cleanup under vitest (globals off) — without this the
// shell renders ACCUMULATE and the role queries find every earlier App
// (the #87 launch-suite lesson, applied here too).
afterEach(cleanup);

describe("the anatomy metrics are the mockup's (#88, from the real stylesheet)", () => {
  it("toolbar 48 / sidebar 210 / rows 36 / status bar 28 / inspector 150 docked / modal 640", () => {
    expect(block(".toolbar")).toMatch(/height:\s*48px/);
    expect(block(".sidebar")).toMatch(/width:\s*210px/);
    expect(block(".statusbar")).toMatch(/height:\s*28px/);
    expect(block(".inspector")).toMatch(/height:\s*150px/);
    expect(block(".modal")).toMatch(/width:\s*640px/);
    expect(block(".table td")).toMatch(/height:\s*36px/);
  });

  it("the inspector docks — it pushes content, it never overlays (no fixed positioning)", () => {
    expect(block(".inspector")).not.toMatch(/position:\s*fixed/);
    expect(block(".inspector")).not.toMatch(/position:\s*absolute/);
  });

  it("the drawer is retired — .drawer and .drawer-backdrop are gone from the sheet", () => {
    expect(CSS).not.toMatch(/^\.drawer \{/m);
    expect(CSS).not.toMatch(/^\.drawer-backdrop \{/m);
  });

  it("the shadow law: shadows ride ONLY the floating surfaces (modal / scrim / toast / focus)", () => {
    const offenders: string[] = [];
    for (const [ctx, line] of ruleLines()) {
      if (!/box-shadow/.test(line)) continue;
      if (/--/.test(line)) continue; // custom-property declaration
      if (/(modal|scrim|toast|focus)/.test(ctx)) continue;
      offenders.push(`${ctx} ${line.trim()}`);
    }
    expect(offenders, "shadow-law offenders").toEqual([]);
  });

  it("no backdrop-filter anywhere — the performance contract", () => {
    expect(CSS).not.toMatch(/backdrop-filter/);
  });
});

describe("the anatomy in the rendered shell (#88)", () => {
  it("the toolbar carries NO brand block — the ladder's Chrome rung, enforced in the component", async () => {
    const { default: App } = await import("./App");
    const { container } = render(<App />);
    const header = container.querySelector("header");
    expect(header).not.toBeNull();
    expect(header!.className).toMatch(/toolbar/);
    expect(header!.querySelector(".brand"), "no .brand in the chrome").toBeNull();
    expect(header!.querySelector(".logomark"), "no logomark in the chrome").toBeNull();
    expect(header!.textContent).not.toContain("Internet Upload Manager");
  });

  it("the toolbar commands: New Upload focuses the path field; the settings seam rides along", async () => {
    const { default: App } = await import("./App");
    const { container, findByRole } = render(<App />);
    const fresh = await findByRole("button", { name: "New Upload" });
    fresh.click();
    expect(document.activeElement?.id).toBe("upload-path");
    expect(container.querySelector("#app-settings")).not.toBeNull();
  });

  it("the sidebar library filters the work pane — real filters, real counts", async () => {
    const { default: App } = await import("./App");
    const { container, findByRole, findAllByRole } = render(<App />);
    await findAllByRole("row");
    // the five rungs exist with counts: All 3, Completed 1 …
    expect(await findByRole("button", { name: /All uploads/ })).toBeTruthy();
    const completed = await findByRole("button", { name: /Completed/ });
    expect(completed.getAttribute("aria-pressed")).toBe("false");
    completed.click();
    await waitFor(() => expect(completed.getAttribute("aria-pressed")).toBe("true"));
    const rows = await findAllByRole("row");
    // header row + the one done job
    expect(rows.length).toBe(2);
    expect(container.querySelector("tbody")!.textContent).toContain("gamma.bin");
    // and All restores the full library
    (await findByRole("button", { name: /All uploads/ })).click();
    await waitFor(() => expect(findAllByRole("row")).resolves.toHaveLength(4));
  });

  it("the status bar speaks the SHORT truth — version yes, raw sentence never", async () => {
    const { default: App } = await import("./App");
    const { container } = render(<App />);
    await waitFor(() => expect(container.querySelector(".statusbar")).not.toBeNull());
    const bar = container.querySelector(".statusbar")!.textContent ?? "";
    expect(bar).toContain("v0.2.1");
    expect(bar).not.toContain("adapters:");
    expect(bar).not.toMatch(/ium-core v/);
  });
});

describe("parseVersion — the status bar's one piece of presentation math", () => {
  it("reads the engine's own sentence, or admits it cannot", async () => {
    const { parseVersion } = await import("./logic");
    expect(parseVersion(ENGINE_SENTENCE)).toBe("v0.2.1");
    expect(parseVersion("no version in here")).toBeNull();
    expect(parseVersion("ium-core v10.20.30 ready")).toBe("v10.20.30");
  });
});
