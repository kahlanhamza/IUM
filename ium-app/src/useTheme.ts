/**
 * The shared theme-preference store (#88): one hook, every instance in
 * sync. The toolbar's compact Appearance control and the Settings card
 * MUST agree — two local `useState`s would drift the moment one moves.
 *
 * theme.ts stays framework-free (its pins run in node and jsdom); this
 * file is the only React-aware seam, a useSyncExternalStore over a
 * module-level listener set. `set` persists, applies the DOM attribute,
 * and notifies every mounted control in the same tick.
 */
import { useSyncExternalStore } from "react";
import {
  applyTheme,
  loadThemePref,
  resolveTheme,
  saveThemePref,
  systemPrefersDark,
  type ThemePref,
} from "./theme";

const listeners = new Set<() => void>();
let cached: ThemePref | null = null;

function getSnapshot(): ThemePref {
  if (cached === null) cached = loadThemePref();
  return cached;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function setPref(pref: ThemePref): void {
  cached = pref;
  saveThemePref(pref);
  applyTheme(resolveTheme(pref, systemPrefersDark()), document);
  for (const l of listeners) l();
}

export function useThemePref(): [ThemePref, (pref: ThemePref) => void] {
  return [useSyncExternalStore(subscribe, getSnapshot), setPref];
}
