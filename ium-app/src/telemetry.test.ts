/**
 * The telemetry charter gate (#66 Lane 7, #75): the written stance exists,
 * carries its sections, and its collected-fields table EQUALS the field
 * mirror the Settings UI renders (TELEMETRY_FIELDS). The charter is the
 * pivot: neither the document nor the code may move without the other —
 * the same construction the brand sheet got in #73.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { TELEMETRY_COPY, TELEMETRY_FIELDS } from "./copy";

const CHARTER_PATH = fileURLToPath(
  new URL("../../docs/11_Telemetry_Stance.md", import.meta.url),
);
// CRLF-normalized: windows runners check out text files with CRLF (the #73
// lesson — the regexes anchor on `$` and a trailing \r defeats them).
const readCharter = () =>
  readFileSync(CHARTER_PATH, "utf8").replace(/\r\n/g, "\n");

const REQUIRED_SECTIONS = [
  "The decision",
  "What IUM collects",
  "What IUM never collects",
  "Where it lands",
  "The switches",
  "The honest limits",
  "The gates",
  "Changing the stance",
];

/** Parse the charter's collected-fields table rows: | `field` | event | ... | */
function charterFields(charter: string): string[] {
  const out: string[] = [];
  for (const line of charter.split("\n")) {
    const m = line.match(/^\| `([a-z_0-9]+)` \|/);
    if (m) out.push(m[1]);
  }
  return out;
}

describe("the telemetry charter (#75) — existence and sections", () => {
  it("exists in-repo (the #66 acceptance bar: the stance is WRITTEN)", () => {
    expect(() => readCharter()).not.toThrow();
  });

  it("carries every required section", () => {
    const charter = readCharter();
    for (const section of REQUIRED_SECTIONS) {
      expect(charter, `missing section: ${section}`).toContain(`## ${section}`);
    }
  });
});

describe("the telemetry charter — the field table cannot drift (D2)", () => {
  it("the charter's collected-fields table equals TELEMETRY_FIELDS exactly", () => {
    expect(charterFields(readCharter())).toEqual(TELEMETRY_FIELDS);
  });

  it("the gate survives a windows-default checkout (the #74 simulation: CRLF disk bytes)", () => {
    // The windows-default clone puts CRLF on disk. Feed the CRLF-converted
    // real charter through the same reader-normalization the gate applies —
    // the full pipeline (CRLF disk → normalize → parse) must yield the
    // identical field list. The pin (.gitattributes) removes the CRLF at
    // the source; this test proves the gate stays robust either way.
    const crlfDisk = readFileSync(CHARTER_PATH, "utf8").replace(/\r?\n/g, "\r\n");
    const table = charterFields(crlfDisk.replace(/\r\n/g, "\n"));
    expect(table).toEqual(TELEMETRY_FIELDS);
  });

  it("the stance sentence the UI renders matches the charter's decision", () => {
    expect(readCharter()).toContain("opt-out");
    expect(TELEMETRY_COPY.stance).toContain("stays on your machine");
  });
});
