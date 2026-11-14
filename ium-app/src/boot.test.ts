/**
 * The boot gate (#112): the UI polls, it never probes once.
 *
 * Tauri builds the config windows BEFORE the shell's `.setup()` manages the
 * engine — every IPC command can answer "state not managed for field
 * 'engine'" during the first seconds after a cold install. The UI must
 * retry within a bounded budget and only then decide: ready, or honestly
 * down. The timing is injected so the contract is pinned without real
 * clocks.
 */
import { describe, expect, it } from "vitest";
import { BOOT_BUDGET_MS, BOOT_INTERVAL_MS, retryUntil, waitForEngine } from "./boot";

/** A virtual clock: `now` reads it, `sleep` advances it — zero real time,
 *  fully deterministic attempt spacing. */
function virtualClock() {
  let t = 0;
  return {
    now: () => t,
    sleep: (ms: number) => {
      t += ms;
      return Promise.resolve();
    },
    time: () => t,
  };
}

describe("retryUntil (#112 — the boot gate)", () => {
  it("answers on the first attempt without ever sleeping", async () => {
    const c = virtualClock();
    let calls = 0;
    const r = await retryUntil(async () => {
      calls++;
      return "ok";
    }, { now: c.now, sleep: c.sleep });
    if (!r.ok) throw new Error("expected an answer");
    expect(r.value).toBe("ok");
    expect(r.attempts).toBe(1);
    expect(c.time()).toBe(0);
  });

  it("retries a boot-race refusal and returns the value — sleeps only BETWEEN attempts", async () => {
    const c = virtualClock();
    let calls = 0;
    const r = await retryUntil(() => {
      calls++;
      if (calls < 3) return Promise.reject(new Error("state not managed for field 'engine'"));
      return Promise.resolve("registry");
    }, { now: c.now, sleep: c.sleep, intervalMs: 10 });
    if (!r.ok) throw new Error("expected an answer after retry");
    expect(r.value).toBe("registry");
    expect(r.attempts).toBe(3);
    expect(c.time()).toBe(20);
  });

  it("reports the last error once the budget is spent — ok: false", async () => {
    const c = virtualClock();
    let calls = 0;
    const r = await retryUntil(() => {
      calls++;
      return Promise.reject(new Error(`down ${calls}`));
    }, { now: c.now, sleep: c.sleep, deadlineMs: 40, intervalMs: 10 });
    expect(r.ok).toBe(false);
    if (r.ok) throw new Error("expected a refusal after the budget");
    // String(Error) prefixes the class — the UI quotes the engine's own
    // words, the test only asserts the message survived verbatim.
    expect(String(r.error)).toContain("down 5");
    expect(r.attempts).toBe(5);
  });

  it("waitForEngine maps the answer onto the boot verdict", async () => {
    const okBoot = await waitForEngine(() => Promise.resolve("ium-core ready"), {
      now: () => 0,
      sleep: async () => {},
    });
    expect(okBoot).toEqual({ ok: true, detail: "ium-core ready" });
    // The DOWN verdict needs a clock that ADVANCES in sleep — a frozen
    // virtual `now` with a noop sleep starves the retry loop forever (the
    // deadline can never pass; the loop is pure microtasks). The real
    // clock advances by itself; the virtual one must too.
    const c = virtualClock();
    const downBoot = await waitForEngine(
      () => Promise.reject(new Error("state not managed")),
      { now: c.now, sleep: c.sleep, deadlineMs: 40, intervalMs: 10 },
    );
    expect(downBoot.ok).toBe(false);
    if (downBoot.ok) throw new Error("expected a down verdict");
    expect(downBoot.detail).toContain("state not managed");
  });

  it("the shared budget is the shell watchdog's number", () => {
    expect(BOOT_BUDGET_MS).toBe(8000);
    expect(BOOT_INTERVAL_MS).toBe(250);
  });
});
