/**
 * The destination picker (the create-form half of the destination lane).
 * RED artifact: at the pre-implementation HEAD the form has no provider
 * select — these queries fail. The contracts:
 *  - "Custom URL" is the DEFAULT and behaves byte-identically to today
 *    (the raw `#upload-endpoint` field stays editable — the power-user
 *    path and the e2e seams are untouched);
 *  - a provider preset swaps the raw field for guided slots + a bucket
 *    field and derives the endpoint into the SAME `#upload-endpoint`
 *    value (readonly — the user sees the truth, the submit path is
 *    unchanged);
 *  - every preset renders its "Get your keys ↗" deep link and recipe.
 */
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

const created = vi.fn();
const createJob = vi.fn((_spec: unknown, _maxConns: number) =>
  Promise.resolve({
    id: "j1",
    source_path: "/x",
    source_size: 1,
    adapter: "s3_multipart",
    endpoint: "https://derived",
    state: "created",
    capability_badge: "resumable" as const,
    created_at: 0,
    updated_at: 0,
  })
);

vi.mock("../api", () => ({
  api: {
    listAdapters: () => Promise.resolve(["http_oneshot", "s3_multipart", "tus"]),
    statFile: () => Promise.resolve(1024),
    createJob: (spec: unknown, maxConns: number) => createJob(spec, maxConns),
  },
}));

import { CreateJobForm } from "./CreateJobForm";

afterEach(() => {
  cleanup();
  created.mockClear();
  createJob.mockClear();
});

function mount() {
  render(<CreateJobForm onCreated={created} recents={[]} onError={() => {}} />);
}

const CUSTOM = "Custom URL (paste any destination)";
const B2 = "Backblaze B2";

/** The e2e seams are IDs (#65) — the tests must speak the same language. */
function byId(id: string): HTMLInputElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} not rendered`);
  return el as HTMLInputElement;
}

async function pickProtocol(name: string) {
  const sel = document.getElementById("upload-adapter") as unknown as HTMLSelectElement;
  await waitFor(() => expect(sel.options.length).toBeGreaterThan(0));
  fireEvent.change(sel, { target: { value: name } });
}

async function pickProvider(label: string) {
  const select = (await waitFor(() => screen.getByLabelText(/Provider/i))) as HTMLSelectElement;
  await waitFor(() => expect(select.options.length).toBeGreaterThan(1));
  fireEvent.change(select, { target: { value: optionValueFor(select, label) } });
}

function optionValueFor(select: HTMLSelectElement, label: string): string {
  const opt = Array.from(select.options).find((o) => o.textContent === label);
  if (!opt) throw new Error(`option '${label}' not offered`);
  return opt.value;
}

describe("the destination picker", () => {
  it("custom is the default and the raw field stays editable", async () => {
    mount();
    const provider = (await waitFor(() => screen.getByLabelText(/Provider/i))) as HTMLSelectElement;
    await waitFor(() => expect(provider.value).not.toBe(""));
    expect(provider.value).toBe(await firstOptionValue(provider, CUSTOM));
    const endpoint = byId("upload-endpoint");
    expect(endpoint.readOnly).toBe(false);
  });

  it("a preset swaps in guided fields, the keys link, and the recipe", async () => {
    mount();
    await pickProtocol("s3_multipart");
    await pickProvider(B2);
    expect(await screen.findByText(/Get your keys/i)).toBeDefined();
    const link = screen.getByText(/Get your keys/i).closest("a");
    expect(link?.getAttribute("href")).toBe("https://backblaze.com/app_keys");
    expect(screen.getByText(/Application Key/i)).toBeDefined(); // the recipe rides the hint
    expect(screen.getByLabelText(/Region/i)).toBeDefined();
    expect(screen.getByLabelText(/Bucket/i)).toBeDefined();
    const endpoint = byId("upload-endpoint");
    expect(endpoint.readOnly).toBe(true);
  });

  it("the derived endpoint tracks bucket and region", async () => {
    mount();
    await pickProtocol("s3_multipart");
    await pickProvider(B2);
    const bucket = await screen.findByLabelText(/Bucket/i);
    fireEvent.change(bucket, { target: { value: "my-bkt" } });
    const endpoint = byId("upload-endpoint");
    expect(endpoint.value).toBe("https://s3.us-west-004.backblazeb2.com/my-bkt");
    fireEvent.change(screen.getByLabelText(/Region/i), { target: { value: "eu-central-003" } });
    expect(endpoint.value).toBe("https://s3.eu-central-003.backblazeb2.com/my-bkt");
  });

  it("back to custom restores the raw field", async () => {
    mount();
    await pickProtocol("s3_multipart");
    await pickProvider(B2);
    fireEvent.change(await screen.findByLabelText(/Bucket/i), { target: { value: "my-bkt" } });
    await pickProvider(CUSTOM);
    const endpoint = byId("upload-endpoint");
    expect(endpoint.readOnly).toBe(false);
    expect(endpoint.value).toBe("");
  });

  it("a finished guided form submits the derived endpoint", async () => {
    mount();
    await pickProtocol("s3_multipart");
    fireEvent.change(byId("upload-path"), { target: { value: "/tmp/video.mp4" } });
    await pickProvider(B2);
    fireEvent.change(await screen.findByLabelText(/Bucket/i), { target: { value: "my-bkt" } });
    const submit = await waitFor(() => {
      const b = document.getElementById("upload-submit") as HTMLButtonElement;
      expect(b.disabled).toBe(false);
      return b;
    });
    fireEvent.click(submit);
    await waitFor(() => expect(createJob).toHaveBeenCalled());
    const spec = createJob.mock.calls[0][0] as { endpoint: string; adapter: string };
    expect(spec.endpoint).toBe("https://s3.us-west-004.backblazeb2.com/my-bkt");
    expect(spec.adapter).toBe("s3_multipart");
  });
});

async function firstOptionValue(select: HTMLSelectElement, label: string): Promise<string> {
  await waitFor(() => expect(select.options.length).toBeGreaterThan(0));
  const opt = Array.from(select.options).find((o) => o.textContent === label);
  if (!opt) throw new Error(`default option '${label}' missing`);
  return opt.value;
}
