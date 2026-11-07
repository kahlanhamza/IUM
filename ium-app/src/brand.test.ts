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
  "What this sheet governs",
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
    const light = cssTokens(css.slice(0, css.indexOf("@media (prefers-color-scheme: dark)")));
    const dark = cssTokens(css.slice(css.indexOf("@media (prefers-color-scheme: dark)")));
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
    const darkStart = css.indexOf("@media (prefers-color-scheme: dark)");
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
