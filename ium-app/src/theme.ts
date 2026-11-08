/**
 * The Ink theme system (#86): the user's light/dark preference.
 *
 * The design law it serves (the owner's ruling): the app carries the logo
 * ink in TWO variants — the near-black ink on light surfaces, the near-white
 * ink on dark — and both modes are first-class. The preference is a UI
 * concern (localStorage, applied before first paint); the engine is never
 * consulted and the choice never blocks boot.
 *
 * CSS contract (theme.css): the LIGHT token set is the `:root` base, the
 * DARK set lives on `[data-theme="dark"]`. Light is expressed by ABSENCE of
 * the attribute, so a no-JS or pre-paint frame renders light, never a
 * broken half-state. `resolveTheme` is the only place the OS signal is
 * interpreted; `applyTheme` is the only place the DOM is touched.
 */

export type ThemePref = "system" | "light" | "dark";

export const THEMES: ThemePref[] = ["system", "light", "dark"];

export const THEME_STORAGE_KEY = "ium.theme";

/** The resolved theme — what the DOM attribute may say. */
export type ResolvedTheme = "light" | "dark";

/** Read the stored preference. Anything but a known value is `system` —
 *  a corrupted write must never throw or produce a third state. */
export function loadThemePref(): ThemePref {
  try {
    const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
    return THEMES.includes(raw as ThemePref) ? (raw as ThemePref) : "system";
  } catch {
    return "system";
  }
}

/** Persist the preference. Storage can be full or blocked (private mode) —
 *  the failure is swallowed because the session still honors the value. */
export function saveThemePref(pref: ThemePref): void {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, pref);
  } catch {
    /* private mode / quota — the in-session choice still applies */
  }
}

/** The preference meets the OS signal. An explicit choice always wins. */
export function resolveTheme(pref: ThemePref, systemDark: boolean): ResolvedTheme {
  if (pref === "system") return systemDark ? "dark" : "light";
  return pref;
}

/** Apply the resolved theme to a document: dark by attribute, light by
 *  absence. Kept element-generic for tests; main.tsx passes `document`. */
export function applyTheme(theme: ResolvedTheme, doc: Pick<Document, "documentElement">): void {
  if (theme === "dark") {
    doc.documentElement.setAttribute("data-theme", "dark");
  } else {
    doc.documentElement.removeAttribute("data-theme");
  }
}

/**
 * Follow the OS signal while the app runs. Returns the unlisten function.
 * In a plain browser without matchMedia (old jsdom), degrades to a no-op
 * so tests and the dev harness never pay for it.
 */
export function watchSystemTheme(onChange: (systemDark: boolean) => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const handler = (e: MediaQueryListEvent) => onChange(e.matches);
  mq.addEventListener?.("change", handler);
  return () => mq.removeEventListener?.("change", handler);
}

/** The OS signal right now (defaults light where matchMedia is absent). */
export function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/**
 * Boot helper: resolve + apply the current theme BEFORE the first React
 * render (no flash), then keep following the OS while the pref is
 * `system`. Returns the unlisten function for teardown.
 */
export function initTheme(): () => void {
  const apply = () => applyTheme(resolveTheme(loadThemePref(), systemPrefersDark()), document);
  apply();
  return watchSystemTheme(() => apply());
}

/**
 * The materials verdict (#92): the engine answers `platform_material` —
 * true only on Windows 11 (OS-build gate + a successful Mica apply) — and
 * this applies it pre-paint, so a Mica boot never flashes opaque. The CSS
 * rewrites ONLY the chrome tokens under `data-material="mica"`; the ink
 * text tokens never move. A false verdict (every other platform, Windows
 * 10, a failed apply) leaves the document untouched — the fully opaque
 * window the e2e lane has always driven.
 */
export function applyPlatformMaterial(mica: boolean, doc: Pick<Document, "documentElement"> = document): void {
  if (mica) doc.documentElement.dataset.material = "mica";
  else delete doc.documentElement.dataset.material;
}
