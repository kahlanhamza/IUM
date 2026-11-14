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
const openSettings = vi.fn();
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
const credentialsSet = vi.fn((_adapter: string, _fields: Record<string, string>) =>
  Promise.resolve()
);
const applyCredentials = vi.fn((_adapter: string) =>
  Promise.resolve({ adapter: "s3_multipart", rebuilt: ["s3_multipart"], removed: [] })
);

/** The adapter fetch is swappable so the boot-race contract (#112) can be
 *  driven: default = the healthy registry. */
const listAdaptersState = vi.hoisted(() => {
  const state: { impl: () => Promise<string[]> } = {
    impl: () => Promise.resolve(["http_oneshot", "s3_multipart", "tus"]),
  };
  return state;
});

/** The capability surface (#119): the vault schema, swappable like the
 *  registry — the fresh-machine contract drives it to the honest empty. */
const schemaState = vi.hoisted(() => {
  const state: { impl: () => Promise<{ adapter: string; label: string }[]> } = {
    impl: () =>
      Promise.resolve([
        { adapter: "s3_multipart", label: "S3 multipart" },
        { adapter: "terabox", label: "TeraBox" },
        { adapter: "mega", label: "MEGA" },
        { adapter: "webdav", label: "WebDAV" },
        { adapter: "dropbox", label: "Dropbox" },
        { adapter: "onedrive", label: "OneDrive" },
      ]),
  };
  return state;
});

vi.mock("../api", () => ({
  api: {
    listAdapters: () => listAdaptersState.impl(),
    credentialsSchema: () => schemaState.impl(),
    statFile: () => Promise.resolve(1024),
    createJob: (spec: unknown, maxConns: number) => createJob(spec, maxConns),
    credentialsSet: (adapter: string, fields: Record<string, string>) =>
      credentialsSet(adapter, fields),
    applyCredentials: (adapter: string) => applyCredentials(adapter),
  },
}));

import { CreateJobForm } from "./CreateJobForm";

/** The boot budget is injected (#112): the retry contract must be provable
 *  without real clocks — 60 ms budget, 5 ms interval, the real helper. */
vi.mock("../boot", async (importOriginal) => ({
  ...await importOriginal<Record<string, unknown>>(),
  BOOT_BUDGET_MS: 60,
  BOOT_INTERVAL_MS: 5,
}));

afterEach(() => {
  cleanup();
  created.mockClear();
  openSettings.mockClear();
  createJob.mockClear();
  credentialsSet.mockClear();
  applyCredentials.mockClear();
  listAdaptersState.impl = () =>
    Promise.resolve(["http_oneshot", "s3_multipart", "tus"]);
});

function mount() {
  render(
    <CreateJobForm
      onCreated={created}
      recents={[]}
      onError={() => {}}
      onOpenSettings={openSettings}
    />,
  );
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

  // #105: the picked region IS the signing scope. The job-time signer reads
  // env → vault, never the job draft — so a guided submit must write the
  // region through to the vault (partial merge: keys untouched) and hot-apply
  // the lane BEFORE createJob drives it. Wasabi rejects a scope that
  // disagrees with the host's region; a guided eu-central-2 pick signed
  // us-east-1 was exactly that rejection.
  it("a guided submit hands the picked region to the vault and hot-applies before the job", async () => {
    mount();
    await pickProtocol("s3_multipart");
    fireEvent.change(byId("upload-path"), { target: { value: "/tmp/video.mp4" } });
    await pickProvider("Wasabi");
    fireEvent.change(screen.getByLabelText(/Region/i), { target: { value: "eu-central-2" } });
    fireEvent.change(await screen.findByLabelText(/Bucket/i), { target: { value: "my-bkt" } });
    const submit = await waitFor(() => {
      const b = document.getElementById("upload-submit") as HTMLButtonElement;
      expect(b.disabled).toBe(false);
      return b;
    });
    fireEvent.click(submit);
    await waitFor(() => expect(createJob).toHaveBeenCalled());
    expect(credentialsSet).toHaveBeenCalledWith("s3_multipart", { region: "eu-central-2" });
    expect(applyCredentials).toHaveBeenCalledWith("s3_multipart");
    const jobOrder = createJob.mock.invocationCallOrder[0];
    expect(credentialsSet.mock.invocationCallOrder[0]).toBeLessThan(jobOrder);
    expect(applyCredentials.mock.invocationCallOrder[0]).toBeLessThan(jobOrder);
  });

  it("a custom submit never writes the vault", async () => {
    mount();
    await pickProtocol("s3_multipart");
    fireEvent.change(byId("upload-path"), { target: { value: "/tmp/video.mp4" } });
    fireEvent.change(byId("upload-endpoint"), { target: { value: "https://s3.example.com/bkt" } });
    const submit = await waitFor(() => {
      const b = document.getElementById("upload-submit") as HTMLButtonElement;
      expect(b.disabled).toBe(false);
      return b;
    });
    fireEvent.click(submit);
    await waitFor(() => expect(createJob).toHaveBeenCalled());
    expect(credentialsSet).not.toHaveBeenCalled();
    expect(applyCredentials).not.toHaveBeenCalled();
  });

  // #112: Tauri boots the webviews before the shell's `.setup()` manages the
  // engine — a fresh install can answer list_adapters with "state not
  // managed" for the first seconds. The form must RETRY, not pin that as a
  // permanent "registry unavailable" banner (the owner's v0.2.4 screenshot).
  it("retries a boot-race refusal — the select fills, no banner, no onError", async () => {
    let calls = 0;
    listAdaptersState.impl = () => {
      calls++;
      if (calls < 3) {
        return Promise.reject(
          new Error("state not managed for field 'engine' on command 'list_adapters'"),
        );
      }
      return Promise.resolve(["http_oneshot", "s3_multipart", "tus"]);
    };
    const onErr = vi.fn();
    render(
      <CreateJobForm
        onCreated={created}
        recents={[]}
        onError={onErr}
        onOpenSettings={openSettings}
      />,
    );
    const sel = document.getElementById("upload-adapter") as unknown as HTMLSelectElement;
    // #119: the fill is the full capability surface — the three REGISTERED
    // values keep their order, the schema lanes follow them.
    await waitFor(
      () => expect(Array.from(sel.options).map((o) => o.value).slice(0, 3)).toEqual(
        ["http_oneshot", "s3_multipart", "tus"],
      ),
      { timeout: 3000 },
    );
    expect(onErr).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("registry unavailable");
    expect(calls).toBe(3);
  });

  it("a registry that never answers within the budget still speaks — once, honestly", async () => {
    let calls = 0;
    listAdaptersState.impl = () => {
      calls++;
      return Promise.reject(
        new Error("state not managed for field 'engine' on command 'list_adapters'"),
      );
    };
    const onErr = vi.fn();
    render(
      <CreateJobForm
        onCreated={created}
        recents={[]}
        onError={onErr}
        onOpenSettings={openSettings}
      />,
    );
    await waitFor(
      () => expect(document.body.textContent).toContain("registry unavailable"),
      { timeout: 3000 },
    );
    expect(onErr).toHaveBeenCalledTimes(1);
    expect(onErr.mock.calls[0][0]).toContain("registry unavailable");
    expect(calls).toBeGreaterThan(1); // it retried before giving up
  });
});

describe("the capability surface (#119): a fresh machine can SEE the destinations", () => {
  /** The owner's fresh Windows machine: zero credential lanes registered —
   *  the registry holds only the generic protocols. */
  function mountFresh() {
    listAdaptersState.impl = () => Promise.resolve(["http_oneshot", "tus"]);
    render(
      <CreateJobForm
        onCreated={created}
        recents={[]}
        onError={() => {}}
        onOpenSettings={openSettings}
      />,
    );
  }

  it("an unregistered lane is selectable, honestly suffixed", async () => {
    mountFresh();
    const sel = document.getElementById("upload-adapter") as unknown as HTMLSelectElement;
    await waitFor(() => expect(sel.options.length).toBeGreaterThan(2));
    const labels = Array.from(sel.options).map((o) => o.textContent);
    expect(labels).toContain("S3 multipart — not connected yet");
    expect(labels).toContain("MEGA — not connected yet");
    expect(labels).toContain("Dropbox — not connected yet");
    fireEvent.change(sel, { target: { value: "s3_multipart" } });
    expect(sel.value).toBe("s3_multipart");
  });

  it("the S3 presets are reachable from an unregistered lane", async () => {
    mountFresh();
    const sel = document.getElementById("upload-adapter") as unknown as HTMLSelectElement;
    await waitFor(() => expect(sel.options.length).toBeGreaterThan(2));
    fireEvent.change(sel, { target: { value: "s3_multipart" } });
    const provider = (await waitFor(() => screen.getByLabelText(/Provider/i))) as HTMLSelectElement;
    const labels = Array.from(provider.options).map((o) => o.textContent);
    expect(labels).toContain(B2);
    expect(labels).toContain("Cloudflare R2");
  });

  it("submit stays disabled and the honest hint opens Settings", async () => {
    mountFresh();
    const sel = document.getElementById("upload-adapter") as unknown as HTMLSelectElement;
    await waitFor(() => expect(sel.options.length).toBeGreaterThan(2));
    fireEvent.change(sel, { target: { value: "mega" } });
    fireEvent.change(byId("upload-path"), { target: { value: "/tmp/video.mp4" } });
    const hint = await waitFor(() => {
      const h = document.getElementById("upload-adapter-hint");
      expect(h).not.toBeNull();
      return h as HTMLElement;
    });
    expect(hint.textContent).toContain("isn\'t connected yet");
    const submit = document.getElementById("upload-submit") as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    const door = screen.getByRole("button", { name: /Open Settings/i });
    fireEvent.click(door);
    expect(openSettings).toHaveBeenCalledTimes(1);
  });

  it("a registered lane is never suffixed — the connected surface is unchanged", async () => {
    // The default (healthy-registry) mount: s3_multipart IS registered.
    mount();
    const sel = document.getElementById("upload-adapter") as unknown as HTMLSelectElement;
    await waitFor(() => expect(sel.options.length).toBeGreaterThan(2));
    const labels = Array.from(sel.options).map((o) => o.textContent);
    expect(labels).toContain("S3 multipart");
    expect(labels).not.toContain("S3 multipart — not connected yet");
    expect(labels).not.toContain("HTTP one-shot — not connected yet");
  });
});


async function firstOptionValue(select: HTMLSelectElement, label: string): Promise<string> {
  await waitFor(() => expect(select.options.length).toBeGreaterThan(0));
  const opt = Array.from(select.options).find((o) => o.textContent === label);
  if (!opt) throw new Error(`default option '${label}' missing`);
  return opt.value;
}
