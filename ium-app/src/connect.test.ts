/**
 * The connect-state contracts (#82) + the Google Drive lane (#84): the
 * one-click connect surface speaks per-lane truth from the same rows the
 * settings math produces. The OAuth backend list now carries DRIVE — the
 * owner lifted the shelving this session ("go ahead with the WebDAV, also
 * with Google drive and YouTube"); YouTube stays OUT (no rclone backend —
 * #85's own native flow). No Tauri imports — plain vitest (the #69 pattern).
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
    hidden: false,
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

describe("the OAuth backend list — the obvious ones, drive joined (#84)", () => {
  it("exactly the three obvious browser-sign-in platforms — dropbox and onedrive went native (#109, #110)", () => {
    expect(OAUTH_BACKENDS.map((b) => b.id)).toEqual(["pcloud", "box", "drive"]);
  });

  it("drive is allowlisted; youtube, dropbox and onedrive are NOT — native flows own them (#85, #109, #110)", () => {
    const ids = OAUTH_BACKENDS.map((b) => b.id);
    expect(ids).toContain("drive");
    for (const absent of ["youtube", "dropbox", "onedrive", "gphotos", "google"]) {
      expect(ids).not.toContain(absent);
    }
  });

  it("isOAuthBackend refuses free-form ids (no injection surface)", () => {
    expect(isOAuthBackend("pcloud")).toBe(true);
    expect(isOAuthBackend("drive")).toBe(true);
    expect(isOAuthBackend("youtube")).toBe(false);
    expect(isOAuthBackend("dropbox")).toBe(false);
    expect(isOAuthBackend("onedrive")).toBe(false);
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

describe("the Google Drive lane — one hidden token row (#84)", () => {
  const token = {
    name: "token",
    label: "Google sign-in",
    secret: true,
    required: true,
    hidden: true,
    present: false,
    saved: false,
    value: null,
    draft: null,
  } as unknown as FieldRow;

  it("not set up until the browser sign-in stores the token", () => {
    expect(connectionState([token])).toBe("empty");
  });

  it("connected the moment the signed-in token is saved — no field wall, ever", () => {
    expect(connectionState([{ ...token, present: true, saved: true }])).toBe("connected");
  });
});
