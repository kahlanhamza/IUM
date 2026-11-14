/**
 * The launch window gate (#87): identity at first paint.
 *
 * The owner asked "what about launching page, is there one?" — there was
 * not: one window, and the first pixels were the working chrome. This gate
 * pins the answer:
 *
 * 1. The WINDOW contract (tauri.conf.json, read off disk): the main window
 *    is declared first (WebDriver handle 0 — the e2e lane attaches there,
 *    not to the splash) and starts HIDDEN; the splash is declared second —
 *    small, frameless, centered, non-resizable, on top, routed to
 *    #/launch. Creation order is the design: the splash shows OVER the
 *    hidden main window, the WebDriver session still lands on main.
 * 2. The SURFACE contract: the dart renders at exactly 96 px — the launch
 *    rung of the logo ladder (the owner's sizing law: where IUM introduces
 *    itself, the mark is BIG) — with the wordmark and ONE live boot
 *    sentence.
 * 3. The HANDOFF contract: the splash completes exactly once, whether the
 *    engine answers or refuses — a down engine must never trap the user at
 *    the splash; the main UI owns the unreachable state honestly.
 */
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";

/** The boot budget is injected (#112): the retry contract must be provable
 *  without real clocks — 60 ms budget, 5 ms interval, the real helper. */
vi.mock("./boot", async (importOriginal) => ({
  ...await importOriginal<Record<string, unknown>>(),
  BOOT_BUDGET_MS: 60,
  BOOT_INTERVAL_MS: 5,
}));

/** Explicit cleanup, not the auto-hook: components from an earlier test
 *  must be UNMOUNTED before the next one counts completions — a still-
 *  mounted splash keeps its own pending handoff and would pollute the
 *  exactly-once contract under test. */
afterEach(() => cleanup());

describe("the launch surface (#87)", () => {
  it("renders the dart at exactly 96 px — the ladder's launch rung", async () => {
    const { mockLaunchDeps, Launch } = await harness();
    mockLaunchDeps({ engine: Promise.resolve("ium-core ready") });
    const { container } = render(<Launch />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("96");
    expect(svg?.getAttribute("height")).toBe("96");
    expect(container.textContent).toContain("IUM");
  });

  it("speaks one boot sentence while starting", async () => {
    const { mockLaunchDeps, Launch, APP_COPY } = await harness();
    mockLaunchDeps({ engine: new Promise(() => {}) }); // never resolves
    const { container } = render(<Launch />);
    expect(container.textContent).toContain(APP_COPY.launchStarting);
  });

  it("completes the handoff once when the engine answers", async () => {
    const { mockLaunchDeps, Launch, completions } = await harness();
    mockLaunchDeps({ engine: Promise.resolve("ium-core ready") });
    render(<Launch />);
    await waitFor(() => expect(completions().length).toBe(1));
  });

  it("completes the handoff even when the engine refuses — never trapped", async () => {
    const { mockLaunchDeps, Launch, completions } = await harness();
    mockLaunchDeps({ engine: Promise.reject(new Error("engine unreachable")) });
    const { container } = render(<Launch />);
    await waitFor(() => expect(completions().length).toBe(1));
    // the honest degraded sentence — the boot says what happened
    await waitFor(() =>
      expect(container.textContent).toContain("opening IUM anyway"),
    );
  });

  // #112: the boot gap is not a down engine. Tauri boots the webviews
  // BEFORE `.setup()` manages the engine, so the splash's first probe can
  // hit "state not managed" while the engine is still opening the journal.
  // The splash must RETRY and complete through the READY path — the
  // owner's v0.2.4 screenshot was a race-lost probe pinned as degraded.
  it("retries a boot-race refusal and hands off through the ready path", async () => {
    const { mockLaunchDeps, Launch, completions } = await harness();
    let calls = 0;
    mockLaunchDeps({
      engine: () => {
        calls++;
        if (calls < 3) return Promise.reject(new Error("state not managed for field 'engine'"));
        return Promise.resolve("ium-core ready");
      },
    });
    const { container } = render(<Launch />);
    await waitFor(() => expect(completions().length).toBe(1), { timeout: 3000 });
    // the READY path — no degraded sentence, ever
    expect(container.textContent).not.toContain("opening IUM anyway");
    expect(calls).toBeGreaterThanOrEqual(3);
  });
});

/** Loads the component lazily (RED reads clearly: missing module), wires
 *  the official Tauri mock harness, and returns the observation points. */
async function harness() {
  const mocks = await import("@tauri-apps/api/mocks");
  const mod = await import("./components/Launch");
  const copy = await import("./copy");
  const calls: string[] = [];
  return {
    Launch: mod.Launch,
    APP_COPY: copy.APP_COPY,
    completions: () => calls,
    mockLaunchDeps(opts: { engine: Promise<string> | (() => Promise<string>) }) {
      calls.length = 0;
      mocks.mockIPC((cmd: string) => {
        if (cmd === "engine_status") {
          return typeof opts.engine === "function" ? opts.engine() : opts.engine;
        }
        if (cmd === "complete_launch") {
          calls.push(cmd);
          return null;
        }
        return null;
      });
    },
  };
}
