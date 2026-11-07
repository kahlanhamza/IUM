/**
 * The settings logic contracts (#69): pure presentation math between the
 * masked IPC view and the form state. The dialog renders these rows; the
 * tests pin the secret-handling rules — the actual credential values never
 * reach the UI, and only EXPLICIT edits travel back.
 */
import { describe, expect, it } from "vitest";
import { canSave, changedFields, toRows, type AdapterViews } from "./settings";

const views: AdapterViews = {
  mega: {
    user: { label: "Email", secret: false, required: true, present: true, value: "u@x", hidden: false },
    pass: { label: "Password", secret: true, required: true, present: true, value: null, hidden: false },
    twofa: { label: "2FA code", secret: true, required: false, present: false, value: null, hidden: false },
  },
  terabox: {
    ndus: { label: "ndus cookie", secret: true, required: true, present: false, value: null, hidden: false },
  },
};

describe("toRows — the masked view becomes a form model", () => {
  it("secrets never carry a value; 'saved' travels as present", () => {
    const rows = toRows(views);
    const pass = rows.mega.find((r) => r.name === "pass")!;
    expect(pass.value).toBeNull();
    expect(pass.present).toBe(true);
    expect(pass.saved).toBe(true);
    expect(pass.draft).toBeNull(); // untouched = not sent back
  });

  it("plain fields carry the stored value for editing", () => {
    const rows = toRows(views);
    const user = rows.mega.find((r) => r.name === "user")!;
    expect(user.value).toBe("u@x");
    expect(user.saved).toBe(true);
    expect(user.draft).toBeNull();
  });

  it("absent fields are neither saved nor draft", () => {
    const rows = toRows(views);
    const ndus = rows.terabox.find((r) => r.name === "ndus")!;
    expect(ndus.present).toBe(false);
    expect(ndus.saved).toBe(false);
    const twofa = rows.mega.find((r) => r.name === "twofa")!;
    expect(twofa.saved).toBe(false);
  });
});

describe("changedFields — only explicit edits travel back", () => {
  it("untouched rows are not sent", () => {
    const rows = toRows(views);
    expect(changedFields(rows.mega)).toEqual({});
  });

  it("typed drafts are sent; explicit clears travel as empty strings", () => {
    const rows = toRows(views);
    rows.mega.find((r) => r.name === "pass")!.draft = "new-secret";
    rows.mega.find((r) => r.name === "twofa")!.draft = "123456";
    expect(changedFields(rows.mega)).toEqual({ pass: "new-secret", twofa: "123456" });
    // The clear affordance: draft "" on a saved field = delete it.
    rows.mega.find((r) => r.name === "pass")!.draft = "";
    expect(changedFields(rows.mega)).toEqual({ pass: "", twofa: "123456" });
  });
});

describe("canSave — the required-field contract", () => {
  it("a required absent field with no draft blocks saving", () => {
    const rows = toRows(views);
    expect(canSave(rows.terabox)).toBe(false);
  });

  it("a drafted value satisfies a required absent field", () => {
    const rows = toRows(views);
    rows.terabox.find((r) => r.name === "ndus")!.draft = "ND";
    expect(canSave(rows.terabox)).toBe(true);
  });

  it("clearing a required present field blocks saving (empty = delete)", () => {
    const rows = toRows(views);
    rows.mega.find((r) => r.name === "pass")!.draft = "";
    expect(canSave(rows.mega)).toBe(false);
  });

  it("an all-optional adapter with nothing filled is savable", () => {
    const rows = toRows(views);
    rows.mega.find((r) => r.name === "user")!.draft = "kept";
    expect(canSave(rows.mega)).toBe(true);
  });
});

describe("hidden rows — the #84 Google Drive token is a chip, not an input", () => {
  const driveViews: AdapterViews = {
    google_drive: {
      token: {
        label: "Google sign-in",
        secret: true,
        required: true,
        present: true,
        value: null,
        hidden: true,
      },
    },
  };

  it("toRows carries the hidden flag from the view", () => {
    const rows = toRows(driveViews);
    const token = rows.google_drive.find((r) => r.name === "token")!;
    expect(token.hidden).toBe(true);
    expect(token.present).toBe(true);
    expect(token.value).toBeNull(); // a secret: the value never travels
  });

  it("a hidden required row counts for canSave exactly like any row", () => {
    const rows = toRows(driveViews);
    expect(canSave(rows.google_drive)).toBe(true); // present = savable
    const cleared = toRows(driveViews);
    cleared.google_drive.find((r) => r.name === "token")!.present = false;
    cleared.google_drive.find((r) => r.name === "token")!.saved = false;
    expect(canSave(cleared.google_drive)).toBe(false); // absent + no draft = blocked
  });
});
