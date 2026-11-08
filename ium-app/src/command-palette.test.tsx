/**
 * The ⌘K command palette (#91) — the keyboard-first lane's contract.
 *
 * The approved mockup (palette-light/dark.html) becomes behavior here: the
 * palette opens on ⌘K / Ctrl+K / the toolbar chip, filters, wraps, runs,
 * closes, and every command it renders is REAL (wired to an API that exists —
 * the UI never lies about what the engine says). The #65-pinned ids and the
 * #88 anatomy pins are untouched: this suite asserts ADDITIONS only.
 */
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, within } from "@testing-library/react";
import "axe-core";

declare global {
  interface Window {
    axe: {
      run: (
        el: HTMLElement,
        ctx?: Record<string, unknown>,
      ) => Promise<{ violations: { id: string; impact: string | null; nodes: unknown[] }[] }>;
    };
  }
}

async function expectAccessible(el: HTMLElement): Promise<void> {
  const { violations } = await window.axe.run(el, {
    rules: { "color-contrast": { enabled: false } },
  });
  const summary = violations
    .map((v) => `${v.id}[${v.impact}] x${v.nodes.length}`)
    .join(", ");
  expect(summary, `axe violations: ${summary}`).toBe("");
}

const ENGINE_SENTENCE =
  "ium-core v0.1.1 ready — adapters: http_one_shot, s3_multipart, tus";

const jobRecord = (id: string, state: string) => ({
  id,
  source_path: `/data/${id}.bin`,
  source_size: 16 * 1024 * 1024,
  adapter: "tus",
  endpoint: "https://x",
  state,
  capability_badge: "resumable" as const,
  created_at: 1,
  updated_at: 2,
});

const pauseJob = vi.fn((_jobId: string) => Promise.resolve());
const resumeSignal = vi.fn((_jobId: string) => Promise.resolve());

vi.mock("./api", () => ({
  api: {
    engineStatus: () => Promise.resolve(ENGINE_SENTENCE),
    listAdapters: () => Promise.resolve(["tus"]),
    listJobs: () =>
      Promise.resolve([
        jobRecord("running-1", "running"),
        jobRecord("paused-1", "paused"),
        jobRecord("done-1", "done"),
      ]),
    snapshot: () => Promise.reject(new Error("unused")),
    statFile: () => Promise.resolve(1048576),
    createJob: () => Promise.reject(new Error("unused")),
    inspectChunks: () => Promise.reject(new Error("unused")),
    pauseJob: (jobId: string) => pauseJob(jobId),
    resumeSignal: (jobId: string) => resumeSignal(jobId),
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
const checkForUpdate = vi.fn(() => Promise.resolve(null));
vi.mock("./updater", () => ({
  checkForUpdate: (...a: unknown[]) => checkForUpdate(...(a as [])),
  updateSupported: () => true,
}));

import App from "./App";

afterEach(() => {
  cleanup();
  pauseJob.mockClear();
  resumeSignal.mockClear();
  checkForUpdate.mockClear();
});

function platform(mac: boolean) {
  Object.defineProperty(navigator, "platform", { value: mac ? "MacIntel" : "Win32", configurable: true });
}

function hotkey(key: string, mods: { meta?: boolean; ctrl?: boolean } = {}) {
  fireEvent.keyDown(window, {
    key,
    metaKey: mods.meta ?? false,
    ctrlKey: mods.ctrl ?? false,
    bubbles: true,
  });
}

async function boot() {
  const tools = render(<App />);
  await tools.findAllByText(/Connected|Connecting/);
  return tools;
}

async function openPalette(mac = true) {
  platform(mac);
  const tools = await boot();
  hotkey("k", { meta: mac, ctrl: !mac });
  return tools;
}

describe("the ⌘K command palette (#91)", () => {
  it("is closed until summoned", async () => {
    await boot();
    expect(document.querySelector('[role="dialog"][aria-label="Command palette"]')).toBeNull();
  });

  it("⌘K opens it, esc closes it, the scrim click closes it", async () => {
    const tools = await openPalette();
    const dlg = await tools.findByRole("dialog", { name: "Command palette" });
    const input = (await tools.findByRole("textbox", { name: "Filter commands" })) as HTMLInputElement;
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Escape", bubbles: true });
    expect(document.querySelector('[aria-label="Command palette"]')).toBeNull();
    // reopen via the same hotkey, close via the scrim this time
    hotkey("k", { meta: true });
    await tools.findByRole("dialog", { name: "Command palette" });
    fireEvent.click(document.querySelector(".cmdk-scrim") as Element);
    expect(document.querySelector('[aria-label="Command palette"]')).toBeNull();
    void dlg;
  });

  it("Ctrl+K opens it on non-mac platforms and the chip shows the platform glyph", async () => {
    const tools = await openPalette(false);
    await tools.findByRole("dialog", { name: "Command palette" });
    const chip = tools.getByRole("button", { name: /command palette/i });
    expect(chip.textContent).toContain("Ctrl");
  });

  it("renders the seven approved commands, grouped, with keycaps", async () => {
    const tools = await openPalette();
    const dlg = await tools.findByRole("dialog", { name: "Command palette" });
    for (const label of [
      /New upload…/,
      /Pause all active uploads/,
      /Resume all paused uploads/,
      /Check for updates/,
      /All uploads/,
      /Connect a destination…/,
      /Open Settings/,
    ]) {
      expect(within(dlg).getAllByRole("option").some((o) => o.textContent?.match(label)),
        `option matching ${label}`).toBe(true);
    }
    within(dlg).getByText("Actions");
    within(dlg).getByText("Go to");
    within(dlg).getByText("⌘N");
    within(dlg).getByText("⌘1");
    within(dlg).getByText("⌘3");
    within(dlg).getByText("⌘,");
  });

  it("typing filters; an unknown query answers honestly", async () => {
    const tools = await openPalette();
    const dlg = await tools.findByRole("dialog", { name: "Command palette" });
    const input = (await tools.findByRole("textbox", { name: "Filter commands" })) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "set" } });
    const options = within(dlg).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(["Open Settings⌘,"]);
    fireEvent.change(input, { target: { value: "zzz" } });
    await tools.findByText("No matching commands");
  });

  it("arrows wrap and Enter runs the active command — New upload… lands on #upload-path", async () => {
    const tools = await openPalette();
    const dlg = await tools.findByRole("dialog", { name: "Command palette" });
    const input = await tools.findByRole("textbox", { name: "Filter commands" });
    // first row active by default
    const options = within(dlg).getAllByRole("option");
    expect(options[0].getAttribute("aria-selected")).toBe("true");
    // ArrowUp wraps to the LAST row
    fireEvent.keyDown(input, { key: "ArrowUp", bubbles: true });
    const after = within(dlg).getAllByRole("option");
    expect(after[after.length - 1].getAttribute("aria-selected")).toBe("true");
    // ArrowDown wraps back to the first; Enter runs it
    fireEvent.keyDown(input, { key: "ArrowDown", bubbles: true });
    fireEvent.keyDown(input, { key: "Enter", bubbles: true });
    expect(document.getElementById("upload-path")).toBeDefined();
    expect(document.activeElement).toBe(document.getElementById("upload-path"));
    // running a command closes the palette
    expect(document.querySelector('[aria-label="Command palette"]')).toBeNull();
  });

  it("Open Settings runs from the palette", async () => {
    const tools = await openPalette();
    const input = await tools.findByRole("textbox", { name: "Filter commands" });
    fireEvent.change(input, { target: { value: "settings" } });
    fireEvent.keyDown(input, { key: "Enter", bubbles: true });
    await tools.findByRole("dialog", { name: "Settings — connections" });
  });

  it("Pause all active uploads: one pause_job per active job, never the paused or done ones", async () => {
    const tools = await openPalette();
    const input = await tools.findByRole("textbox", { name: "Filter commands" });
    fireEvent.change(input, { target: { value: "pause" } });
    fireEvent.keyDown(input, { key: "Enter", bubbles: true });
    await vi.waitFor(() => expect(pauseJob).toHaveBeenCalledTimes(1));
    expect(pauseJob).toHaveBeenCalledWith("running-1");
    expect(resumeSignal).not.toHaveBeenCalled();
  });

  it("Resume all paused uploads: one resume_signal per paused job", async () => {
    const tools = await openPalette();
    const input = await tools.findByRole("textbox", { name: "Filter commands" });
    fireEvent.change(input, { target: { value: "resume" } });
    fireEvent.keyDown(input, { key: "Enter", bubbles: true });
    await vi.waitFor(() => expect(resumeSignal).toHaveBeenCalledTimes(1));
    expect(resumeSignal).toHaveBeenCalledWith("paused-1");
    expect(pauseJob).not.toHaveBeenCalled();
  });

  it("batch honesty: with zero candidates the palette says so instead of pretending", async () => {
    const listJobs = vi.fn(() => Promise.resolve([jobRecord("done-1", "done")]));
    const apiModule = (await import("./api")) as { api: Record<string, unknown> };
    const original = apiModule.api.listJobs;
    apiModule.api.listJobs = listJobs as never;
    try {
      const tools = await openPalette();
      const input = await tools.findByRole("textbox", { name: "Filter commands" });
      fireEvent.change(input, { target: { value: "pause" } });
      fireEvent.keyDown(input, { key: "Enter", bubbles: true });
      await tools.findByText("No active uploads to pause.");
    } finally {
      apiModule.api.listJobs = original;
    }
  });

  it("Check for updates speaks the honest outcome as a toast", async () => {
    const tools = await openPalette();
    const input = await tools.findByRole("textbox", { name: "Filter commands" });
    fireEvent.change(input, { target: { value: "update" } });
    fireEvent.keyDown(input, { key: "Enter", bubbles: true });
    await vi.waitFor(() => expect(checkForUpdate).toHaveBeenCalledTimes(1));
    await tools.findByText("IUM is up to date.");
  });

  it("the toolbar Commands chip opens the palette and shows the platform glyph", async () => {
    platform(true);
    const tools = await boot();
    const chip = tools.getByRole("button", { name: /command palette/i });
    expect(chip.textContent).toContain("⌘");
    fireEvent.click(chip);
    await tools.findByRole("dialog", { name: "Command palette" });
  });

  it("⌘N and ⌘, are global reflexes — they work with the palette closed", async () => {
    platform(true);
    const tools = await boot();
    hotkey("n", { meta: true });
    expect(document.activeElement).toBe(document.getElementById("upload-path"));
    hotkey(",", { meta: true });
    await tools.findByRole("dialog", { name: "Settings — connections" });
  });

  it("focus returns to the summoner on close", async () => {
    platform(true);
    const tools = await boot();
    const settingsBtn = tools.getByRole("button", { name: /Open settings/ });
    settingsBtn.focus();
    hotkey("k", { meta: true });
    const input = await tools.findByRole("textbox", { name: "Filter commands" });
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Escape", bubbles: true });
    expect(document.activeElement).toBe(settingsBtn);
  });

  it("has no axe violations with the palette open", async () => {
    const tools = await openPalette();
    const dlg = await tools.findByRole("dialog", { name: "Command palette" });
    await expectAccessible(dlg);
  });
});
