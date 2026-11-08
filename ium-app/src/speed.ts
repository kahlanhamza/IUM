/**
 * The speed-cap preference (#94): the user's global upload cap, persisted
 * shell-side. The cap is a POLICY KNOB in the engine (runtime state, not
 * journal truth — the set_policy family's shape), so the shell owns the
 * persistence: one localStorage key, re-asserted at every boot. The stored
 * unit is KB/s (what the user types); the engine's unit is bytes/sec — the
 * conversion lives here, once, so both call sites agree.
 */

export const RATE_CAP_STORAGE_KEY = "ium.rate_cap_kbps";

/** The persisted choice: `null` = unlimited (the key's absence, "0", or
 *  garbage all mean unlimited — a preference is never a crash). */
export function loadRateCapKbps(): number | null {
  try {
    const raw = window.localStorage.getItem(RATE_CAP_STORAGE_KEY);
    if (raw === null) return null;
    const n = Number(raw);
    if (!Number.isFinite(n) || n <= 0) return null;
    return Math.floor(n);
  } catch {
    return null;
  }
}

/** Persist the choice (KB/s; `null` clears the key = unlimited). */
export function saveRateCapKbps(kbps: number | null): void {
  try {
    if (kbps === null || kbps <= 0) {
      window.localStorage.removeItem(RATE_CAP_STORAGE_KEY);
    } else {
      window.localStorage.setItem(RATE_CAP_STORAGE_KEY, String(Math.floor(kbps)));
    }
  } catch {
    /* persistence is best-effort; the session knob still applies */
  }
}

/** KB/s (the user's unit) → bytes/sec (the engine's unit). */
export function kbpsToBytesPerSec(kbps: number): number {
  return Math.floor(kbps * 1024);
}
