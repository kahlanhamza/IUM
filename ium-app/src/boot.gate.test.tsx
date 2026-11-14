/**
 * The shell boot gate (#112): nothing loads until the engine answers.
 *
 * RED at the pre-fix HEAD: App.tsx fired engineStatus / list_jobs /
 * credentials probes as ONE-SHOT mount effects. Tauri boots the webviews
 * BEFORE the shell's `.setup()` manages the engine, so on a cold install
 * every probe can land inside the gap and answer "state not managed for
 * field 'engine' on command '...'". The UI then pinned the failure
 * permanently: red "Engine unreachable" pill, "No uploads yet", the
 * "registry unavailable" banner — the owner's v0.2.4 first-launch
 * screenshot, while the engine actually came up a second later.
 *
 * The contract: the shell waits for the engine (bounded retries), and the
 * journal reads fire only AFTER the engine has answered. A budget spent
 * without an answer renders the honest down state — and a background
 * recovery poll may still flip it to ready.
 */
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";

const state = vi.hoisted(() => ({
  engineCalls: 0,
  engineFailsLeft: 0,
  listJobsCalls: 0,
}));

/** The boot budget is injected: 60 ms budget, 5 ms interval, the real helper. */
vi.mock("./boot", async (importOriginal) => ({
  ...await importOriginal<Record<string, unknown>>(),
  BOOT_BUDGET_MS: 60,
  BOOT_INTERVAL_MS: 5,
}));

vi.mock("./api", () => ({
  api: {
    engineStatus: () => {
      state.engineCalls++;
      if (state.engineFailsLeft > 0) {
        state.engineFailsLeft--;
        return Promise.reject(
          new Error("state not managed for field 'engine' on command 'engine_status'"),
        );
      }
      return Promise.resolve("ium-core v0.2.4 ready — adapters: http_one_shot, s3_multipart, tus");
    },
    listJobs: () => {
      state.listJobsCalls++;
      return Promise.resolve([]);
    },
    listAdapters: () => Promise.resolve(["http_oneshot"]),
    snapshot: () => Promise.reject(new Error("unused")),
    statFile: () => Promise.resolve(0),
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
    notifyUser: () => Promise.resolve(false),
    setRateCap: () => Promise.resolve(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// jsdom has no auto-cleanup under vitest (globals off) — without this the
// shell renders ACCUMULATE (the #87 lesson, applied here too).
afterEach(cleanup);

function reset(failsLeft: number) {
  state.engineCalls = 0;
  state.engineFailsLeft = failsLeft;
  state.listJobsCalls = 0;
}

describe("the shell boot gate (#112)", () => {
  it("list_jobs fires only AFTER the engine has answered the boot", async () => {
    reset(2); // two race-lost probes, then the engine lands
    const { default: App } = await import("./App");
    render(<App />);
    await waitFor(() => expect(state.listJobsCalls).toBeGreaterThan(0), { timeout: 3000 });
    // the answer came from a RETRY — attempt 3, not the race-lost first probe
    expect(state.engineCalls).toBeGreaterThanOrEqual(3);
  });

  it("a boot that resolves paints the ready pill", async () => {
    reset(1);
    const { default: App } = await import("./App");
    render(<App />);
    await waitFor(
      () => expect(document.body.textContent).toContain("Connected"),
      { timeout: 3000 },
    );
    expect(document.body.textContent).not.toContain("Engine unreachable");
  });

  it("a budget spent without an answer renders the honest down state", async () => {
    reset(9999); // never answers within the injected budget
    const { default: App } = await import("./App");
    render(<App />);
    await waitFor(
      () => expect(document.body.textContent).toContain("Engine unreachable"),
      { timeout: 3000 },
    );
    // the journal was never read — the boot gate holds
    expect(state.listJobsCalls).toBe(0);
    expect(state.engineCalls).toBeGreaterThan(1); // it retried before giving up
  });
});
