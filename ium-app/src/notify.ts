/**
 * The terminal-notification module (#93): the OS notification surface for
 * the two terminal job states (done / failed).
 *
 * Two halves, both tiny:
 *
 * 1. The PURE decision — `notifyDraftFor(prev, record, options)`. The shell
 *    feeds it every job-state change it sees (the live `job_updated`
 *    listener, the 4 s poll, and a manual refresh all land in the same
 *    `jobs` array, so ONE detector watching that array covers every path
 *    with no double fires). It answers with a draft or null; every rule of
 *    the behavior matrix lives here and is pinned in notify.test.ts:
 *
 *    - first sight of a job (no prev) → null — a boot re-listing must not
 *      spam the OS surface;
 *    - a transition INTO a terminal state (done / failed) → one draft;
 *    - same-state re-emits → null (idempotent);
 *    - the toggle off → null; a focused window → null (the in-app toast
 *      already spoke — the OS surface is for the away case).
 *
 * 2. The persisted preference — the theme store's exact shape (module-level
 *    store + useSyncExternalStore + localStorage), so the Settings card's
 *    toggle and the watcher read the same truth. The default is ON: the
 *    feature exists to be used; one switch turns it off.
 *
 * The SEND itself is injected (a `Notifier`), so this module stays
 * runtime-free: vitest needs no Tauri, and a plain-browser dev run no-ops.
 */

import { useSyncExternalStore } from "react";
import { NOTIFICATIONS_COPY, humanizeError } from "./copy";

/** What the OS notification shows — the whole design face, in words. */
export interface NotifyDraft {
  title: string;
  body: string;
}

/** A record's shape the watcher needs — the JobRecord fields verbatim. */
export interface NotifyRecord {
  id: string;
  source_path: string;
  adapter: string;
  state: string;
}

export interface NotifyOptions {
  /** The Settings card's persisted switch. */
  enabled: boolean;
  /** `document.hasFocus()` at the transition — focused means silent. */
  focused: boolean;
  /** The engine's own words for a failure, when the journal carried one. */
  raw?: string;
}

export const NOTIFY_STORAGE_KEY = "ium.notify.enabled";
const TERMINAL_STATES = new Set(["done", "failed"]);

/** The file's basename only — never the machine's path layout. Handles the
 * POSIX and Windows separators the engine's source paths can carry. */
export function basenameOf(path: string): string {
  const norm = path.replace(/\\/g, "/");
  const base = norm.slice(norm.lastIndexOf("/") + 1);
  return base || path;
}

/**
 * The transition rule (pure). `prev` is the state the shell last saw for
 * this job id — `undefined` on first sight. Returns the draft to show, or
 * null for "nothing".
 */
export function notifyDraftFor(
  prev: string | undefined,
  record: NotifyRecord,
  options: NotifyOptions,
): NotifyDraft | null {
  if (!options.enabled || options.focused) return null;
  if (prev === undefined) return null;
  if (prev === record.state) return null;
  if (!TERMINAL_STATES.has(record.state)) return null;
  const name = basenameOf(record.source_path);
  if (record.state === "done") {
    return {
      title: NOTIFICATIONS_COPY.doneTitle,
      body: NOTIFICATIONS_COPY.doneBody(name, record.adapter),
    };
  }
  const cause = options.raw ? humanizeError(options.raw).title : null;
  return {
    title: NOTIFICATIONS_COPY.failedTitle,
    body: NOTIFICATIONS_COPY.failedBody(name, cause),
  };
}

// --- the persisted preference (the theme store's shape, one concern) ------

let pref: boolean = loadNotifyPref();
const listeners = new Set<() => void>();

export function loadNotifyPref(): boolean {
  try {
    const raw = window.localStorage.getItem(NOTIFY_STORAGE_KEY);
    return raw === null ? true : raw === "1";
  } catch {
    return true;
  }
}

/** Flip the switch: persists, then notifies every mounted control. */
export function setNotifyPref(value: boolean): void {
  pref = value;
  try {
    window.localStorage.setItem(NOTIFY_STORAGE_KEY, value ? "1" : "0");
  } catch {
    /* persistence is best-effort; the in-memory value still rules */
  }
  for (const l of listeners) l();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): boolean {
  return pref;
}

/** The Settings card's toggle binding. */
export function useNotifyPref(): [boolean, (value: boolean) => void] {
  return [useSyncExternalStore(subscribe, getSnapshot), setNotifyPref];
}
