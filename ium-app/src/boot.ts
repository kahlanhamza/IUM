/**
 * The boot gate (#112): the UI polls, it never probes once.
 *
 * Tauri builds the config windows BEFORE the shell's `.setup()` runs
 * (tauri 2.12.5, `crates/tauri/src/app.rs` `fn setup`), so both IUM
 * webviews load React while the engine is still installing telemetry,
 * applying the vault, and opening the SQLite WAL journal — the Engine
 * state is managed only at the END of setup. Every IPC command landing
 * inside that gap answers `state not managed for field 'engine' on
 * command '...'`; extraction fails before our code runs, so no command
 * can answer gracefully. The UI must therefore RETRY within a bounded
 * budget and only then decide: ready, or honestly down.
 *
 * The budget is the shell watchdog's own number (8 s, #87) and the
 * timing is injectable so the contract is provable without real clocks.
 */

/** The boot budget: the shell-side launch watchdog's number (#87, #112). */
export const BOOT_BUDGET_MS = 8000;
/** The retry cadence: fast enough to feel instant, slow enough to be
 *  invisible next to a 30 s cold boot. */
export const BOOT_INTERVAL_MS = 250;

export interface RetryOpts {
  /** Total budget from the first attempt (default BOOT_BUDGET_MS). */
  deadlineMs?: number;
  /** Delay between attempts, never before the first (default BOOT_INTERVAL_MS). */
  intervalMs?: number;
  /** Injected clock — virtual in tests. */
  now?: () => number;
  /** Injected sleep — advances the virtual clock in tests. */
  sleep?: (ms: number) => Promise<void>;
}

export type RetryOutcome<T> =
  | { ok: true; value: T; attempts: number }
  | { ok: false; error: unknown; attempts: number };

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms));

/** Run `task` on a fixed cadence until it answers or the budget is spent.
 *  The first attempt fires immediately; the interval applies only BETWEEN
 *  attempts; the budget is measured from the first attempt's start. The
 *  last refusal is preserved verbatim — the UI's honest words come from
 *  the engine's own error. */
export async function retryUntil<T>(
  task: () => Promise<T>,
  opts: RetryOpts = {},
): Promise<RetryOutcome<T>> {
  const deadlineMs = opts.deadlineMs ?? BOOT_BUDGET_MS;
  const intervalMs = opts.intervalMs ?? BOOT_INTERVAL_MS;
  const now = opts.now ?? Date.now;
  const sleep = opts.sleep ?? defaultSleep;
  const start = now();
  let attempts = 0;
  let lastError: unknown;
  for (;;) {
    attempts++;
    try {
      return { ok: true, value: await task(), attempts };
    } catch (e) {
      lastError = e;
    }
    // The next attempt must still land within the budget to be worth its sleep.
    if (now() - start + intervalMs > deadlineMs) {
      return { ok: false, error: lastError, attempts };
    }
    await sleep(intervalMs);
  }
}

export type EngineBoot =
  | { ok: true; detail: string }
  | { ok: false; detail: string };

/** The engine verdict: `engine_status` retried until it answers or the
 *  budget is spent. `detail` carries the engine's own sentence either way
 *  — the pill and the toast quote the engine, they never invent. */
export async function waitForEngine(
  status: () => Promise<string>,
  opts: RetryOpts = {},
): Promise<EngineBoot> {
  const r = await retryUntil(status, opts);
  return r.ok ? { ok: true, detail: r.value } : { ok: false, detail: String(r.error) };
}
