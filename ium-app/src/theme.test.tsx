/**
 * The Ink theme system (#86): the user-facing theme preference.
 *
 * Three contracts, all pinned here:
 * 1. The preference math is pure — system | light | dark resolves against
 *    the OS signal, the stored value can never produce a third state.
 * 2. The preference persists in localStorage under ONE key, validated on
 *    read (a corrupted value falls back to system, never throws).
 * 3. Applying a theme writes exactly one attribute on the document root —
 *    no class churn, no inline styles, no flash (main.tsx applies before
 *    the first render).
 *
 * The owner's ruling this lane implements: the app carries the logo ink in
 * TWO variants — the dark ink on light surfaces, the light ink on dark —
 * and both must be first-class modes, not a dark afterthought.
 */
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import {
  applyTheme,
  loadThemePref,
  resolveTheme,
  saveThemePref,
  THEME_STORAGE_KEY,
  THEMES,
  type ThemePref,
} from "./theme";

describe("theme preference math (#86)", () => {
  it("exposes exactly the three preferences", () => {
    expect(THEMES).toEqual(["system", "light", "dark"]);
  });

  it("system follows the OS signal both ways", () => {
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("system", true)).toBe("dark");
  });

  it("an explicit choice always wins over the OS signal", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});

describe("theme persistence (#86)", () => {
  it("round-trips the preference under one storage key", () => {
    // jsdom localStorage
    window.localStorage.clear();
    saveThemePref("dark");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(loadThemePref()).toBe("dark");
    saveThemePref("light");
    expect(loadThemePref()).toBe("light");
    saveThemePref("system");
    expect(loadThemePref()).toBe("system");
  });

  it("a corrupted stored value degrades to system", () => {
    window.localStorage.clear();
    window.localStorage.setItem(THEME_STORAGE_KEY, "not-a-theme");
    expect(loadThemePref()).toBe("system");
    window.localStorage.setItem(THEME_STORAGE_KEY, '{"json":"garbage"}');
    expect(loadThemePref()).toBe("system");
  });
});

describe("theme application (#86)", () => {
  it("applies dark by attribute and light by absence (light is the base)", () => {
    document.documentElement.removeAttribute("data-theme");
    applyTheme("dark", document);
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    applyTheme("light", document);
    expect(document.documentElement.getAttribute("data-theme")).toBeNull();
  });

  it("never leaves a stale attribute behind", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    applyTheme("light", document);
    expect(document.documentElement.getAttribute("data-theme")).toBeNull();
  });
});

describe("the Appearance control (#86)", () => {
  it("renders three choices with the current one selected", async () => {
    const { renderThemeToggle } = await testHarness();
    const current = vi.fn();
    const { container } = renderThemeToggle({ value: "system", onChange: current });
    const radios = container.querySelectorAll<HTMLInputElement>('input[type="radio"]');
    expect(radios.length).toBe(3);
    const checked = Array.from(radios).find((r) => r.checked);
    expect(checked?.value).toBe("system");
  });

  it("fires onChange with the chosen preference", async () => {
    const { renderThemeToggle } = await testHarness();
    const onChange = vi.fn();
    const { container } = renderThemeToggle({ value: "system", onChange });
    const dark = container.querySelector<HTMLInputElement>('input[value="dark"]');
    dark!.click();
    expect(onChange).toHaveBeenCalledWith("dark");
  });

  it("labels the group for assistive tech", async () => {
    const { renderThemeToggle } = await testHarness();
    const { container } = renderThemeToggle({ value: "light", onChange: () => {} });
    const group = container.querySelector('[role="radiogroup"]');
    expect(group?.getAttribute("aria-label")).toBeTruthy();
  });
});

/** Loads the component lazily so the pure contracts above fail first and
 *  read clearly at RED time. */
async function testHarness() {
  const mod = await import("./components/ThemeToggle");
  const { render } = await import("@testing-library/react");
  return {
    renderThemeToggle: (props: {
      value: ThemePref;
      onChange: (p: ThemePref) => void;
    }) => render(mod.ThemeToggle(props)),
  };
}
