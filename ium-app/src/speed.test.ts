// @vitest-environment jsdom
//
// The speed-cap shell pins (#94): the persistence helpers and the Settings
// card's behavior. The ENGINE pins (the bucket's virtual-time pacing, the
// between-lease acquire, byte-exactness under a cap) live in
// ium-core/tests/rate_cap.rs; this file pins the shell half of the contract.
import { afterEach, describe, expect, it, vi } from "vitest";
import { SPEED_COPY } from "./copy";
import {
  kbpsToBytesPerSec,
  loadRateCapKbps,
  RATE_CAP_STORAGE_KEY,
  saveRateCapKbps,
} from "./speed";

afterEach(() => {
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("the speed cap's shell half (#94)", () => {
  it("absent, zero, negative, and garbage all mean unlimited — never a crash", () => {
    expect(loadRateCapKbps()).toBeNull();
    for (const raw of ["0", "-5", "abc", "Infinity", "1e999"]) {
      window.localStorage.setItem(RATE_CAP_STORAGE_KEY, raw);
      expect(loadRateCapKbps()).toBeNull();
    }
  });

  it("a real choice round-trips through localStorage", () => {
    saveRateCapKbps(512);
    expect(window.localStorage.getItem(RATE_CAP_STORAGE_KEY)).toBe("512");
    expect(loadRateCapKbps()).toBe(512);
    saveRateCapKbps(null);
    expect(window.localStorage.getItem(RATE_CAP_STORAGE_KEY)).toBeNull();
    expect(loadRateCapKbps()).toBeNull();
  });

  it("a fractional choice floors to a whole KB/s", () => {
    saveRateCapKbps(12.9);
    expect(loadRateCapKbps()).toBe(12);
  });

  it("the user's unit converts once, here: KB/s → bytes/sec", () => {
    expect(kbpsToBytesPerSec(1)).toBe(1024);
    expect(kbpsToBytesPerSec(512)).toBe(512 * 1024);
  });

  it("the card's copy states the honest contract (total across uploads, hot)", () => {
    expect(SPEED_COPY.contract).toMatch(/total across every upload and lane/);
    expect(SPEED_COPY.contract).toMatch(/running uploads included/);
    expect(SPEED_COPY.switchLabel).toBe("Limit total upload speed");
  });

  it("localStorage failures degrade to the session value, never a throw", () => {
    const original = window.localStorage.setItem;
    vi.spyOn(window.localStorage, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    expect(() => saveRateCapKbps(512)).not.toThrow();
    void original;
  });
});
