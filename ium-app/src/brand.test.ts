/**
 * The brand gate (#66 Lane 6, #73): the brand sheet exists, cannot drift
 * from the design tokens it documents, and the UI copy follows the voice
 * it declares. The sheet is a PROJECTION of theme.css — not a parallel
 * truth; the copy surface is sentences, not shouty strings.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { APP_COPY, DEFAULT_COPY, RULES, TELEMETRY_COPY, type CopyRule } from "./copy";
import { ADAPTER_LABELS } from "./logic";

// node:fs is the ONE mechanism for both files: vitest stubs plain `.css`
// imports to "" even with `?raw`, so the real file must be read off disk.
// Both readers normalize CRLF: windows runners check out text files with
// CRLF (no .gitattributes pin — #74), and the table regexes anchor on `$`.
const SHEET_PATH = fileURLToPath(
  new URL("../../docs/10_Brand_Sheet.md", import.meta.url),
);
const THEME_PATH = fileURLToPath(new URL("./theme.css", import.meta.url));
/** The dark scheme's selector at line start — the split point the token
 *  projection reads (NOT a bare substring: the design-law comment in
 *  theme.css mentions the attribute mid-sentence). */
const DARK_SELECTOR = '\n[data-theme="dark"]';
const readSheet = () =>
  readFileSync(SHEET_PATH, "utf8").replace(/\r\n/g, "\n");
const readTheme = () =>
  readFileSync(THEME_PATH, "utf8").replace(/\r\n/g, "\n");

const REQUIRED_SECTIONS = [
  "What IUM is",
  "Name and wordmark",
  "Voice",
  "Naming rules",
  "Color",
  "Type",
  "Iconography",
  "Logo scale ladder",
  "What this sheet governs",
];

/** The logo ladder (#86, the owner's sizing principle): the mark scales
 *  with the identity weight of the surface — BIG where IUM introduces
 *  itself, absent where the work is the hero. Five rungs, exact sizes. */
const LOGO_LADDER: Array<[string, string]> = [
  ["Launch", "96"],
  ["About", "64"],
  ["First run and empty states", "48"],
  ["Chrome", "none"],
  ["README and docs", "128"],
];

/** The five vendored font files the type system rides (latin subsets). */
const BRAND_FONTS: Array<[string, string, string]> = [
  ["Inter", "400", "/fonts/inter-latin-400-normal.woff2"],
  ["Inter", "500", "/fonts/inter-latin-500-normal.woff2"],
  ["Inter", "600", "/fonts/inter-latin-600-normal.woff2"],
  ["JetBrains Mono", "400", "/fonts/jetbrains-mono-latin-400-normal.woff2"],
  ["JetBrains Mono", "500", "/fonts/jetbrains-mono-latin-500-normal.woff2"],
];

/** The hex color tokens the sheet's color table must carry, both schemes. */
const BRAND_COLOR_TOKENS = [
  "--bg",
  "--surface",
  "--surface-2",
  "--surface-3",
  "--text",
  "--text-2",
  "--text-3",
  "--border",
  "--border-strong",
  "--accent",
  "--accent-hover",
  "--accent-down",
  "--accent-text",
  "--accent-subtle",
  "--ok",
  "--ok-bg",
  "--danger",
  "--danger-bg",
  "--warn",
  "--warn-bg",
  "--info",
  "--info-bg",
];

describe("the brand sheet (#73) — existence and sections", () => {
  it("exists in-repo (the #66 acceptance bar: a brand sheet exists)", () => {
    expect(() => readSheet()).not.toThrow();
  });

  it("carries every required section", () => {
    const sheet = readSheet();
    for (const section of REQUIRED_SECTIONS) {
      expect(sheet, `missing section: ${section}`).toContain(`## ${section}`);
    }
  });
});

describe("the brand sheet — the token projection cannot drift (D2)", () => {
  /** Parse `--name: value;` declarations from a CSS chunk. */
  function cssTokens(css: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const m of css.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)) {
      out[`--${m[1]}`] = m[2].trim();
    }
    return out;
  }

  /** Parse the sheet's color table rows: | `--tok` | `#hex` | `#hex` | */
  function sheetTable(sheet: string): Record<string, [string, string]> {
    const out: Record<string, [string, string]> = {};
    for (const line of sheet.split("\n")) {
      const m = line.match(/^\| `(--[a-z0-9-]+)` \| `(#[0-9a-fA-F]{6})` \| `(#[0-9a-fA-F]{6})` \|$/);
      if (m) out[m[1]] = [m[2], m[3]];
    }
    return out;
  }

  it("the sheet documents every brand color token for BOTH schemes", () => {
    const table = sheetTable(readSheet());
    const missing = BRAND_COLOR_TOKENS.filter((t) => !(t in table));
    expect(missing, "tokens missing from the sheet table").toEqual([]);
  });

  it("the gate survives a windows-default checkout (the #74 simulation: CRLF disk bytes)", () => {
    // The windows-default clone (core.autocrlf=true, no pin) puts CRLF on
    // disk. This feeds the CRLF-converted real files through the SAME
    // reader-normalization the gate applies — the full pipeline (CRLF disk
    // → normalize → parse) must produce the identical tables. The pin
    // (.gitattributes) removes the CRLF-at-the-source; this test proves the
    // gate stays robust either way — the #74 acceptance line.
    const crlfDisk = (s: string) => s.replace(/\r?\n/g, "\r\n"); // the checkout writes CRLF
    const gateNormalize = (s: string) => s.replace(/\r\n/g, "\n"); // the readers' normalization
    const rawSheet = readFileSync(SHEET_PATH, "utf8");
    const rawTheme = readFileSync(THEME_PATH, "utf8");

    const table = sheetTable(gateNormalize(crlfDisk(rawSheet)));
    const missing = BRAND_COLOR_TOKENS.filter((t) => !(t in table));
    expect(missing, "tokens missing from the CRLF sheet table").toEqual([]);

    const css = gateNormalize(crlfDisk(rawTheme));
    const split = css.indexOf(DARK_SELECTOR);
    const light = cssTokens(css.slice(0, split));
    const dark = cssTokens(css.slice(split));
    const drift: string[] = [];
    for (const tok of BRAND_COLOR_TOKENS) {
      const row = table[tok];
      expect(row, `${tok} missing from the CRLF sheet`).toBeDefined();
      if (light[tok] !== row[0]) drift.push(`${tok} light: sheet ${row[0]} ≠ css ${light[tok]}`);
      if (dark[tok] !== row[1]) drift.push(`${tok} dark: sheet ${row[1]} ≠ css ${dark[tok]}`);
    }
    expect(drift, "the CRLF pipeline drifted from theme.css").toEqual([]);
  });

  it("the sheet's hex values equal theme.css's actual values — light and dark", () => {
    const css = readTheme();
    const darkStart = css.indexOf(DARK_SELECTOR);
    expect(darkStart).toBeGreaterThan(0);
    const light = cssTokens(css.slice(0, darkStart));
    const dark = cssTokens(css.slice(darkStart));
    const table = sheetTable(readSheet());

    const drift: string[] = [];
    for (const tok of BRAND_COLOR_TOKENS) {
      const row = table[tok];
      expect(row, `${tok} missing from the sheet`).toBeDefined();
      if (light[tok] !== row[0]) drift.push(`${tok} light: sheet ${row[0]} ≠ css ${light[tok]}`);
      if (dark[tok] !== row[1]) drift.push(`${tok} dark: sheet ${row[1]} ≠ css ${dark[tok]}`);
    }
    expect(drift, "the sheet has drifted from theme.css").toEqual([]);
  });
});

describe("the brand sheet — the logo scale ladder (#86)", () => {
  /** Parse ladder rows: | Rung name | 96 px | — sizes as written. */
  function ladderTable(sheet: string): Record<string, string> {
    const out: Record<string, string> = {};
    const section = sheet.split("## Logo scale ladder")[1] ?? "";
    for (const line of section.split("\n")) {
      const m = line.match(/^\| `?([A-Za-z][^`|]*?)`? \| `?([0-9]+|none)`?(?: px)?`? \|$/);
      if (m) out[m[1].trim()] = m[2];
    }
    return out;
  }

  it("pins every rung of the logo ladder at its exact size", () => {
    const ladder = ladderTable(readSheet());
    const missing = LOGO_LADDER.filter(([rung]) => !(rung in ladder));
    expect(missing, "ladder rungs missing from the sheet").toEqual([]);
    const drift = LOGO_LADDER.filter(
      ([rung, size]) => ladder[rung] !== undefined && ladder[rung] !== size,
    );
    expect(drift.map(([r, s]) => `${r}: sheet ${ladder[r]} ≠ gate ${s}`)).toEqual([]);
  });

  it("carries the ladder rule as a sentence (identity surfaces go bigger)", () => {
    const section = readSheet().split("## Logo scale ladder")[1] ?? "";
    expect(section.length).toBeGreaterThan(200);
  });
});

describe("the Ink type system (#86) — vendored Inter + JetBrains Mono", () => {
  /** Escape a literal for RegExp use (paths carry dots). */
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

  it("theme.css declares every brand face as @font-face over the vendored files", () => {
    const css = readTheme();
    const missing = BRAND_FONTS.filter(([family, weight, path]) => {
      const re = new RegExp(
        `@font-face\\s*{[^}]*font-family:\\s*["']?${esc(family)}["']?;` +
          `[^}]*font-style:\\s*normal;[^}]*font-weight:\\s*${weight};` +
          `[^}]*src:\\s*url\\(["']?${esc(path)}`,
      );
      return !re.test(css);
    });
    expect(missing, "font faces missing from theme.css").toEqual([]);
  });

  it("the vendored woff2 files exist on disk and are real (not placeholders)", () => {
    const root = fileURLToPath(new URL("../public/", import.meta.url));
    const tiny = BRAND_FONTS.filter(([, , p]) => {
      const bytes = readFileSync(root + p.replace(/^\//, "")).byteLength;
      return bytes < 10_000; // a latin-subset woff2 is 25-50 KB
    });
    expect(tiny, "font files missing or implausibly small").toEqual([]);
  });

  it("the UI and mono stacks lead with the brand faces, OS faces stay as fallback", () => {
    const css = readTheme();
    expect(css).toMatch(/--font:\s*["']Inter["'],\s*system-ui/);
    expect(css).toMatch(/--mono:\s*["']JetBrains Mono["'],\s*ui-monospace/);
  });
});

describe("the Ink theme attribute (#86) — both modes are first-class", () => {
  it("the dark scheme is an attribute block, not only a media query", () => {
    const css = readTheme();
    expect(css).toContain('[data-theme="dark"]');
  });

  it("light stays the base on :root (no attribute needed, no flash)", () => {
    const css = readTheme();
    const root = css.slice(0, css.indexOf(DARK_SELECTOR));
    expect(root).toMatch(/--bg:/);
    expect(root).toMatch(/--text:/);
  });
});

describe("the brand sheet — the UI copy follows the voice (D3)", () => {
  /** The whole enumerated sentence surface: copy rules + default + app copy. */
  function sentences(): { text: string; where: string }[] {
    const out: { text: string; where: string }[] = [];
    RULES.forEach((r: CopyRule, i: number) => {
      out.push({ text: r.title, where: `RULES[${i}].title` });
      out.push({ text: r.hint, where: `RULES[${i}].hint` });
    });
    out.push({ text: DEFAULT_COPY.title, where: "DEFAULT_COPY.title" });
    out.push({ text: DEFAULT_COPY.hint, where: "DEFAULT_COPY.hint" });
    for (const [k, v] of Object.entries(APP_COPY)) {
      out.push({ text: v, where: `APP_COPY.${k}` });
    }
    // The telemetry Privacy surface rides the same voice (#75).
    for (const [k, v] of Object.entries(TELEMETRY_COPY)) {
      out.push({ text: v, where: `TELEMETRY_COPY.${k}` });
    }
    return out;
  }

  it("every sentence starts capitalized (calm, specific, human)", () => {
    const bad = sentences().filter((s) => !/^[A-Z0-9"'(]/.test(s.text));
    expect(bad.map((b) => `${b.where}: ${b.text}`)).toEqual([]);
  });

  it("no exclamation marks anywhere in the copy surface", () => {
    const bad = sentences().filter((s) => s.text.includes("!"));
    expect(bad.map((b) => `${b.where}: ${b.text}`)).toEqual([]);
  });

  it("no emoji anywhere in the copy surface (structured badges stay replaced)", () => {
    const emoji = /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/u;
    const bad = sentences().filter((s) => emoji.test(s.text));
    expect(bad.map((b) => `${b.where}: ${b.text}`)).toEqual([]);
  });

  it("the name is spelled IUM wherever it appears — never Ium or ium", () => {
    const bad = sentences().filter((s) => /ium/i.test(s.text) && !/IUM/.test(s.text));
    expect(bad.map((b) => `${b.where}: ${b.text}`)).toEqual([]);
  });

  it("adapter labels are friendly — no raw seam ids leak into the table", () => {
    const bad = Object.entries(ADAPTER_LABELS).filter(
      ([seam, label]) => !label || label.includes("_") || label === seam,
    );
    expect(bad).toEqual([]);
  });
});
