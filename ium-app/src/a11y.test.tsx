/**
 * The a11y contract (#66 Lane 1): axe-core over every real screen state,
 * rendered in jsdom with journal-shaped fixtures. This is a GATE, not a
 * report — one new violation fails `npm test` in CI, so the product cannot
 * silently regress to a shell only mouse users can drive.
 *
 * Scope honesty: jsdom cannot compute styles, so axe's color-contrast rule
 * comes back "incomplete" (never fails here). Structural accessibility —
 * names, roles, keyboard reachability, landmarks, ARIA correctness — is
 * fully enforced. The visual contrast bar is pinned separately on #66 as a
 * lane (token pairs are fixed, so the check is a design-system invariant).
 */
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import "axe-core";

declare global {
  // axe-core's UMD build attaches itself to window once imported above.
  interface Window {
    axe: {
      run: (
        el: HTMLElement,
        ctx?: Record<string, unknown>,
      ) => Promise<{ violations: { id: string; impact: string | null; nodes: unknown[] }[] }>;
    };
  }
}

async function expectAccessible(el: HTMLElement): Promise<void> {
  const { violations } = await window.axe.run(el, {
    // color-contrast needs real layout — jsdom reports it "incomplete",
    // which axe surfaces as a violation only when `resultTypes` includes
    // it; excluded here and pinned as the design-system lane on #66.
    rules: { "color-contrast": { enabled: false } },
  });
  const summary = violations
    .map((v) => `${v.id}[${v.impact}] x${v.nodes.length}`)
    .join(", ");
  expect(summary, `axe violations: ${summary}`).toBe("");
}

import type { JobRecord, JobSnapshot } from "./api";
import { CreateJobForm } from "./components/CreateJobForm";
import { InspectorDrawer } from "./components/InspectorDrawer";
import { JobsTable } from "./components/JobsTable";
import { SettingsDialog } from "./components/SettingsDialog";

const job = (over: Partial<JobRecord>): JobRecord => ({
  id: "job-1",
  source_path: "/data/quarterly-report.pdf",
  source_size: 16 * 1024 * 1024,
  adapter: "tus",
  endpoint: "https://uploads.example.com/files",
  state: "running",
  capability_badge: "resumable",
  created_at: 1,
  updated_at: 2,
  ...over,
});

const snapshot: JobSnapshot = {
  job: job({}),
  chunks: [
    { idx: 0, begin: 0, end: 4194304, state: "verified", bytes_confirmed: 4194304, adapter_token: "tok-0" },
    { idx: 1, begin: 4194304, end: 8388608, state: "uploaded", bytes_confirmed: 8388608, adapter_token: "tok-1" },
    { idx: 2, begin: 8388608, end: 12582912, state: "leased", bytes_confirmed: 0, adapter_token: null },
    { idx: 3, begin: 12582912, end: 16777216, state: "pending", bytes_confirmed: 0, adapter_token: null },
  ],
  events: [
    [1, "job_created", '{"plan":4}'],
    [2, "replan_cap", '{"cap":4194304}'],
  ],
};

describe("a11y: the intake form (error state included — registry failure is deterministic in jsdom)", () => {
  it("has no axe violations", async () => {
    const { container } = render(
      <CreateJobForm onCreated={() => {}} recents={["https://uploads.example.com/files"]} onError={() => {}} />,
    );
    await expectAccessible(container as HTMLElement);
  });
});

describe("a11y: the jobs table (running / paused / done)", () => {
  it("has no axe violations", async () => {
    const { container } = render(
      <JobsTable
        jobs={[
          { record: job({ id: "a" }), confirmed: 4 * 1024 * 1024, speed: 262144 },
          { record: job({ id: "b", state: "paused" }), confirmed: 1024, speed: 0 },
          { record: job({ id: "c", state: "done" }), confirmed: 16 * 1024 * 1024, speed: 0 },
        ]}
        selected={null}
        onSelect={() => {}}
        onPause={() => {}}
        onResume={() => {}}
        onCancel={() => {}}
      />,
    );
    await expectAccessible(container as HTMLElement);
  });
});

describe("a11y: the inspector drawer (dialog + chunk table + journal)", () => {
  it("has no axe violations", async () => {
    const { container } = render(<InspectorDrawer snapshot={snapshot} onClose={() => {}} />);
    await expectAccessible(container as HTMLElement);
  });
});

describe("a11y: the full shell (header, pill, form, table, empty state)", () => {
  it("has no axe violations — POPULATED, not empty: the first harness run audited the empty state and missed the table (#67 harness gap)", async () => {
    vi.mock("./api", () => ({
      api: {
        engineStatus: () => Promise.resolve("journal ok — 3 jobs"),
        listAdapters: () => Promise.resolve(["http", "tus", "s3", "feishu", "terabox", "mega"]),
        listJobs: () =>
          Promise.resolve([
            job({ id: "a" }),
            job({ id: "b", state: "paused" }),
            job({ id: "c", state: "done" }),
          ]),
        statFile: () => Promise.resolve(1048576),
        snapshot: () => Promise.resolve(snapshot),
        createJob: () => Promise.reject(new Error("unused in audit")),
        inspectChunks: () => Promise.resolve(snapshot),
        pauseJob: () => Promise.resolve(),
        resumeSignal: () => Promise.resolve(),
        cancelJob: () => Promise.reject(new Error("unused in audit")),
        // The credentials vault (#69): schema + masked views.
        credentialsSchema: () =>
          Promise.resolve([
            {
              adapter: "mega",
              label: "MEGA (rclone bridge)",
              note: "Your MEGA login.",
              fields: [
                { name: "user", label: "Email", secret: false, required: true },
                { name: "pass", label: "Password", secret: true, required: true },
              ],
            },
            {
              adapter: "terabox",
              label: "TeraBox",
              note: "The ndus cookie.",
              fields: [{ name: "ndus", label: "ndus cookie", secret: true, required: true }],
            },
          ]),
        credentialsGet: () =>
          Promise.resolve({
            mega: {
              user: { label: "Email", secret: false, required: true, present: true, value: "u@x" },
              pass: { label: "Password", secret: true, required: true, present: true, value: null },
            },
            terabox: {
              ndus: { label: "ndus cookie", secret: true, required: true, present: false, value: null },
            },
          }),
        credentialsSet: () => Promise.resolve(),
        credentialsDelete: () => Promise.resolve(),
        applyCredentials: () => Promise.resolve({ action: "replaced", lanes: [] }),
        probeCredentials: () =>
          Promise.resolve({ checked: false, ok: true, sentence: "", final: true }),
        secretMode: () => Promise.resolve("keyring"),
        // The telemetry Privacy card (#75): the audit sees the consent
        // switch ON with a populated local buffer — the trust surface.
        telemetryStatus: () =>
          Promise.resolve({
            consent: true,
            count: 2,
            path: "/home/u/.local/share/ium/telemetry.jsonl",
          }),
        telemetrySetConsent: () => Promise.resolve(),
        telemetryClear: () => Promise.resolve(),
      },
    }));
    vi.mock("@tauri-apps/api/event", () => ({
      listen: () => Promise.resolve(() => {}),
    }));
    const { default: App } = await import("./App");
    const { container, findAllByText } = render(<App />);
    // The shell's state resolves over async IPC — audit the POPULATED
    // product (table rows + named progressbars), not the boot frame.
    // (Three fixture rows share the file name — findAll, not find.)
    await findAllByText("quarterly-report.pdf");
    await expectAccessible(container as HTMLElement);
  });
});

describe("a11y: the settings dialog (#69 — the credentials vault, masked)", () => {
  it("has no axe violations with saved secrets shown as chips, not values", async () => {
    const { container, findByLabelText } = render(
      <SettingsDialog open onClose={() => {}} onError={() => {}} onSaved={() => {}} />,
    );
    // Wait for the async schema/views pull: the Email field carries the
    // stored value; the Password field must show NO value at all.
    // Exact label text ("Field *" = required marker) — the reveal buttons'
    // aria-labels ("Reveal Password") must not collide with the query.
    const email = (await findByLabelText("Email *")) as HTMLInputElement;
    expect(email.value).toBe("u@x");
    const pass = (await findByLabelText("Password *")) as HTMLInputElement;
    expect(pass.value).toBe(""); // a secret never travels to the UI
    expect(pass.type).toBe("password"); // and is masked by default
    await expectAccessible(container as HTMLElement);
  });
});
