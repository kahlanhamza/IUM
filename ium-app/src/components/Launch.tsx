/**
 * The launch surface (#87): the first pixels IUM ever shows.
 *
 * The design law it serves is the logo ladder's TOP rung — the owner's
 * sizing principle: "where we introduce our software to users the logo
 * must be bigger." The dart stands at exactly 96 px with the wordmark
 * under it and ONE live boot sentence; nothing else competes.
 *
 * The handoff is honest by construction: the sentence reports the real
 * boot phase (the engine answering — or refusing), and the completion
 * fires EITHER WAY. A down engine never traps the user at the splash; the
 * main window owns the unreachable state (its own honest pill), the
 * watchdog in the shell (`complete_launch`'s sibling timer) backs the same
 * contract from the Rust side. No fake progress, no animation library.
 *
 * #112: the probe is a POLL, not a one-shot. Tauri boots the webviews
 * before the shell's `.setup()` manages the engine, so the first probe
 * can hit "state not managed" while the engine is still opening the
 * journal — a boot gap is not a down engine, and one race-lost refusal
 * must not pin the splash as degraded (the owner's v0.2.4 screenshot).
 * The verdict is the shared boot budget's (BOOT_BUDGET_MS): answered →
 * the ready handoff; spent → the honest degraded handoff. Same
 * never-trap contract as before — now decided after real retries.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { api } from "../api";
import { BOOT_BUDGET_MS, BOOT_INTERVAL_MS, waitForEngine } from "../boot";
import { APP_COPY } from "../copy";
import { Logomark } from "./icons";

export function Launch() {
  // string on purpose: the sentence changes as the boot resolves (the
  // literal from APP_COPY must not become the state type).
  const [line, setLine] = useState<string>(APP_COPY.launchStarting);

  useEffect(() => {
    // The guard is per effect run, and the handoff timer is cleanup-owned:
    // StrictMode's dev double-invoke runs this effect twice — run 1's
    // pending handoff is cancelled by cleanup, run 2 owns the real one —
    // so production (one run), development (two runs) and an unmount
    // mid-boot all end with AT MOST one live handoff and no zombie invoke.
    let done = false;
    let handoff: number | undefined;

    const complete = (sentence?: string) => {
      if (done) return;
      done = true;
      if (sentence) setLine(sentence);
      // Let the final sentence paint before the window swaps — one beat,
      // not a timer theater.
      handoff = window.setTimeout(() => {
        void invoke("complete_launch").catch(() => {
          /* plain-browser dev has no shell — the page is the whole story */
        });
      }, 350);
    };

    void waitForEngine(() => api.engineStatus(), {
      deadlineMs: BOOT_BUDGET_MS,
      intervalMs: BOOT_INTERVAL_MS,
    }).then((boot) => {
      // ready → the plain handoff; budget spent → the honest degraded
      // sentence. Either way the handoff fires exactly once (`done`).
      complete(boot.ok ? undefined : APP_COPY.launchDegraded);
    });

    // The IPC-side safety net: if the shell never answers either way
    // (a wedged engine mid-boot), the splash still yields.
    const watchdog = window.setTimeout(
      () => complete(APP_COPY.launchDegraded),
      8000,
    );
    return () => {
      // #112: the boot verdict may resolve AFTER the splash is gone (the
      // retry budget outlives an unmount in tests and in a fast handoff).
      // `done = true` retires the verdict — an unmounted splash never
      // completes, so no zombie invoke can fire (the exactly-once
      // contract covers the whole lifetime, not just mounted ones).
      done = true;
      window.clearTimeout(watchdog);
      if (handoff !== undefined) window.clearTimeout(handoff);
    };
  }, []);

  return (
    <div className="launch">
      <Logomark className="launch-mark" size={96} />
      <div className="launch-name">IUM</div>
      <div className="launch-sub">Internet Upload Manager</div>
      <div className="launch-status" role="status" aria-live="polite">
        {line}
      </div>
    </div>
  );
}
