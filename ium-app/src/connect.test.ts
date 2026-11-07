/**
 * The connect-state contracts (#82): the one-click connect surface speaks
 * per-lane truth from the same rows the settings math produces. The OAuth
 * backend list pins the owner's shelving: Google types appear nowhere
 * (#59/#60 stay shelved) — the obvious browser-sign-in platforms only.
 * No Tauri imports — plain vitest (the #69 pattern).
 */
import { describe, expect, it } from "vitest";
import { connectionState, isOAuthBackend, OAUTH_BACKENDS } from "./connect";
import type { FieldRow } from "./settings";

function row(partial: Partial<FieldRow> & { name: string }): FieldRow {
  return {
    label: partial.name,
    secret: false,
    required: false,
    present: false,
    saved: false,
    value: null,
    draft: null,
    ...partial,
  };
}

describe("connectionState — the chip tells the per-lane truth", () => {
  it("empty: no required field is saved", () => {
    const rows = [
      row({ name: "user", required: true }),
      row({ name: "pass", required: true, secret: true }),
    ];
    expect(connectionState(rows)).toBe("empty");
  });

  it("partial: some required saved, some missing", () => {
    const rows = [
      row({ name: "user", required: true, present: true, saved: true, value: "u@x" }),
      row({ name: "pass", required: true, secret: true }),
    ];
    expect(connectionState(rows)).toBe("partial");
  });

  it("connected: every required field is saved (optional absent is fine)", () => {
    const rows = [
      row({ name: "user", required: true, present: true, saved: true, value: "u@x" }),
      row({ name: "pass", required: true, secret: true, present: true, saved: true }),
      row({ name: "twofa", secret: true }),
    ];
    expect(connectionState(rows)).toBe("connected");
  });

  it("drafts are not saved values — typed-but-unsaved is not connected", () => {
    const rows = [row({ name: "user", required: true, draft: "u@x" })];
    expect(connectionState(rows)).toBe("empty");
  });
});

describe("the OAuth backend list — the obvious ones, Google stays shelved", () => {
  it("exactly the four obvious browser-sign-in platforms", () => {
    expect(OAUTH_BACKENDS.map((b) => b.id)).toEqual(["pcloud", "dropbox", "onedrive", "box"]);
  });

  it("no Google type appears — #59/#60 are owner-shelved", () => {
    const ids = OAUTH_BACKENDS.map((b) => b.id);
    for (const shelved of ["drive", "gdrive", "youtube", "gphotos", "google"]) {
      expect(ids).not.toContain(shelved);
    }
  });

  it("isOAuthBackend refuses free-form ids (no injection surface)", () => {
    expect(isOAuthBackend("pcloud")).toBe(true);
    expect(isOAuthBackend("drive")).toBe(false);
    expect(isOAuthBackend("$(rm -rf)")).toBe(false);
    expect(isOAuthBackend("")).toBe(false);
  });

  it("every backend carries a label and a sign-in sentence", () => {
    for (const b of OAUTH_BACKENDS) {
      expect(b.label.length).toBeGreaterThan(0);
      expect(b.note.endsWith(".")).toBe(true);
    }
  });
});
