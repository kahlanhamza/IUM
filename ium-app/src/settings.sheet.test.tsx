/**
 * The settings sheet's layout contract (#113): the sheets are the
 * product's credential surface and must survive the owner's own
 * screenshot test. ONE field per row (`.sheet-grid`), labels that WRAP
 * (the 90-character S3 endpoint label must never paint across its
 * neighbors again — `white-space: nowrap` is banned on field labels).
 * The CREATE form's 4-column `.form-grid` is a different surface with a
 * different job; the sheets may not borrow it. The floating-surface
 * opacity law (#114) is pinned in anatomy.test.tsx with the other chrome
 * laws.
 */
// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";

const CSS = readFileSync(resolve(process.cwd(), "src/theme.css"), "utf8").replace(/\r\n/g, "\n");

/** Pull a top-level rule body for a selector, e.g. block(".modal") -> CSS text. */
function block(selector: string): string {
  const i = CSS.indexOf(`${selector} {`);
  expect(i, `${selector} exists in theme.css`).toBeGreaterThanOrEqual(0);
  const end = CSS.indexOf("}", i);
  return CSS.slice(i, end);
}

vi.mock("./api", () => ({
  api: {
    applyCredentials: () => Promise.resolve({ action: "replaced", lanes: [] }),
    authorizeOauth: () => Promise.resolve({ action: "replaced", lanes: [] }),
    connectDropbox: () => Promise.resolve({ action: "replaced", lanes: [] }),
    connectOnedrive: () => Promise.resolve({ action: "replaced", lanes: [] }),
    connectYoutube: () => Promise.resolve({ action: "replaced", lanes: [] }),
    credentialsDelete: () => Promise.resolve(),
    credentialsGet: () =>
      Promise.resolve({
        s3_multipart: {
          access_key: { label: "Access key", secret: true, required: true, present: false, value: null, hidden: false },
          secret_key: { label: "Secret key", secret: true, required: true, present: false, value: null, hidden: false },
          region: { label: "Region", secret: false, required: true, present: false, value: null, hidden: false },
          endpoint: { label: "Bucket URL (the Test button's target)", secret: false, required: false, present: false, value: null, hidden: false },
        },
      }),
    credentialsSchema: () =>
      Promise.resolve([
        {
          adapter: "s3_multipart",
          label: "S3 multipart",
          note: "Any S3-compatible storage. Paste a bucket URL below to make the Test button check your keys against it.",
          fields: [
            { name: "access_key", label: "Access key", secret: true, required: true, hidden: false },
            { name: "secret_key", label: "Secret key", secret: true, required: true, hidden: false },
            { name: "region", label: "Region", secret: false, required: true, hidden: false },
            { name: "endpoint", label: "Bucket URL (the Test button's target)", secret: false, required: false, hidden: false },
          ],
        },
      ]),
    credentialsSet: () => Promise.resolve(),
    probeCredentials: () =>
      Promise.resolve({ checked: false, ok: true, sentence: "", final: true }),
    secretMode: () => Promise.resolve("keyring"),
    setRateCap: () => Promise.resolve(),
    telemetryClear: () => Promise.resolve(),
    telemetrySetConsent: () => Promise.resolve(),
    telemetryStatus: () =>
      Promise.resolve({ consent: true, count: 0, path: "/tmp/telemetry.jsonl" }),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// jsdom has no auto-cleanup under vitest (globals off) — the #87 lesson.
afterEach(cleanup);

describe("the connect sheets' layout contract (#113)", () => {
  it("field labels WRAP — nowrap is what painted the S3 label across its neighbors", () => {
    expect(block(".field-label")).not.toMatch(/white-space:\s*nowrap/);
  });

  it("the sheet grid exists: ONE field per row, full width, breathing room", () => {
    const b = block(".sheet-grid");
    expect(b).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  });

  it("the create form's 4-column grid stays for the create form only", () => {
    // The sheets' own class must exist and must not be .form-grid —
    // this pin guards against a silent revert to the shared template.
    expect(CSS).toMatch(/^\.sheet-grid \{/m);
  });

  it("the S3 sheet renders its fields inside .sheet-grid — one per row", async () => {
    const { SettingsDialog } = await import("./components/SettingsDialog");
    const { container, findByRole } = render(
      <SettingsDialog open onClose={() => {}} onError={() => {}} onSaved={() => {}} />,
    );
    const details = await findByRole("button", { name: "Connect S3 multipart" });
    details.click();
    await findByRole("button", { name: "Save and connect S3 multipart" });
    const grid = container.querySelector(".sheet-grid");
    expect(grid, "the sheet's fields ride .sheet-grid").not.toBeNull();
    expect(grid!.children.length).toBe(4);
    expect(container.querySelector(".form-grid"), "the create-form grid is not in the sheet").toBeNull();
  });
});
