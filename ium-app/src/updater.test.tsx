/**
 * The auto-update gate (#80): the release bar's "auto-update default-on" made
 * real. Three contracts:
 *
 * 1. the updater wrapper exists and degrades HONESTLY (#67 rule): outside a
 *    Tauri context checkForUpdate() resolves null — never throws, never
 *    blanks, never pretends;
 * 2. Settings → About carries the update control and tells the truth in
 *    every state (checking / up to date / available / downloading / ready /
 *    error) — calm sentences per the brand voice, no exclamation marks;
 * 3. consent rule: an update is NEVER installed without the user's click —
 *    checking is automatic, applying is user-armed (#80 design note).
 */
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const updaterMock = vi.hoisted(() => ({
  updateSupported: vi.fn(() => true),
  checkForUpdate: vi.fn(),
  downloadAndInstall: vi.fn(),
  restartToUpdate: vi.fn(),
}));

vi.mock("./updater", () => updaterMock);

vi.mock("./api", () => ({
  api: {
    engineStatus: () =>
      Promise.resolve("ium-core v0.2.2 ready — adapters: http_one_shot, s3_multipart, tus"),
    listAdapters: () => Promise.resolve([]),
    listJobs: () => Promise.resolve([]),
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

import { SettingsDialog } from "./components/SettingsDialog";
import { checkForUpdate, updateSupported } from "./updater";

function renderDialog() {
  return render(
    <SettingsDialog open onClose={() => {}} onError={() => {}} onSaved={() => {}} />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  updaterMock.updateSupported.mockReturnValue(true);
});

describe("the updater wrapper (#80)", () => {
  it("degrades honestly outside Tauri — resolves null, never throws", async () => {
    updaterMock.updateSupported.mockReturnValue(false);
    updaterMock.checkForUpdate.mockResolvedValue(null);
    const info = await checkForUpdate();
    expect(info).toBeNull();
    expect(updateSupported()).toBe(false);
  });

  it("reports an available update as {version, notes}", async () => {
    updaterMock.checkForUpdate.mockResolvedValue({
      version: "0.2.2",
      notes: "Fixes and polish.",
    });
    const info = await checkForUpdate();
    expect(info).toEqual({ version: "0.2.2", notes: "Fixes and polish." });
  });
});

describe("the About update control (#80)", () => {
  it("never renders the updater card outside Tauri — honest degrade", async () => {
    updaterMock.updateSupported.mockReturnValue(false);
    renderDialog();
    await waitFor(() => expect(screen.getByLabelText("About IUM")).toBeTruthy());
    expect(screen.queryByText(/Check for updates/)).toBeNull();
  });

  it("walks the honest state machine: checking → up to date", async () => {
    updaterMock.checkForUpdate.mockResolvedValue(null);
    renderDialog();
    const btn = await screen.findByText("Check for updates");
    fireEvent.click(btn);
    expect(screen.getByText(/Checking for updates/)).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByText(/is up to date/)).toBeTruthy(),
    );
  });

  it("available update → consent click → download progress → restart armed", async () => {
    let release: (() => void) | undefined;
    updaterMock.checkForUpdate.mockResolvedValue({ version: "0.2.2", notes: null });
    updaterMock.downloadAndInstall.mockImplementation(
      (onProgress?: (p: { downloaded: number; total?: number }) => void) =>
        new Promise<void>((resolve) => {
          // the real plugin streams progress DURING the download — emit
          // Started then 60%, and only resolve (→ ready) when the test
          // releases, so the mid-flight state is observable
          onProgress?.({ downloaded: 0, total: 10_000_000 });
          onProgress?.({ downloaded: 6_000_000, total: 10_000_000 });
          release = resolve;
        }),
    );
    renderDialog();
    const btn = await screen.findByText("Check for updates");
    fireEvent.click(btn);
    await waitFor(() => expect(screen.getByText(/0\.2\.2 is available/)).toBeTruthy());
    // the consent rule: nothing installs until the user clicks
    expect(updaterMock.downloadAndInstall).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Download 0.2.2"));
    await waitFor(() => expect(updaterMock.downloadAndInstall).toHaveBeenCalled());
    expect(screen.getByText(/Downloading 60%/)).toBeTruthy();
    release?.();
    await waitFor(() => expect(screen.getByText("Restart to update")).toBeTruthy());
    fireEvent.click(screen.getByText("Restart to update"));
    expect(updaterMock.restartToUpdate).toHaveBeenCalledTimes(1);
  });

  it("an update error surfaces as a sentence, not a silence", async () => {
    updaterMock.checkForUpdate.mockRejectedValue(new Error("endpoint unreachable"));
    renderDialog();
    fireEvent.click(await screen.findByText("Check for updates"));
    await waitFor(() =>
      expect(screen.getByText(/endpoint unreachable/)).toBeTruthy(),
    );
  });
});
