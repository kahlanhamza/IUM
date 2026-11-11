/**
 * The provider registry loader (the destination lane, owner-ratified
 * 2026-11-11). RED artifact: `./providers` does not exist at the
 * pre-implementation HEAD — this file fails the suite until the loader
 * lands. The registry JSON (`src/data/providers.json`) is the single
 * source of truth; the Rust cross-check (`ium-core/tests/
 * providers_registry.rs`) feeds every sample URL through the ENGINE's own
 * `parse_target`, so what the picker derives can never be a destination
 * the engine refuses. These tests pin the TS-side contract: schema
 * coherence, derivation math, and the custom-mode escape hatch.
 */
import { describe, expect, it } from "vitest";
import {
  PROVIDER_CUSTOM,
  buildEndpoint,
  canDerive,
  loadProviders,
  providerById,
  providersForAdapter,
} from "./providers";

const B2 = () =>
  providerById("backblaze-b2") ??
  (() => {
    throw new Error("backblaze-b2 preset missing");
  })();

describe("the provider registry", () => {
  it("loads at least the launch set", () => {
    expect(loadProviders().length).toBeGreaterThanOrEqual(6);
  });

  it("rides the s3_multipart lane only (v1 scope)", () => {
    for (const p of loadProviders()) expect(p.adapter).toBe("s3_multipart");
  });

  it("ids are unique kebab-case", () => {
    const ids = loadProviders().map((p) => p.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z0-9-]+$/);
  });

  it("carries a key recipe for every preset", () => {
    for (const p of loadProviders()) {
      expect(p.key_steps.trim().split(/\s+/).length).toBeGreaterThanOrEqual(5);
      expect(p.key_page === null || p.key_page.startsWith("https://")).toBe(true);
    }
  });

  it("providersForAdapter leads with custom, then the lane's presets", () => {
    const s3 = providersForAdapter("s3_multipart");
    expect(s3.length).toBe(loadProviders().length + 1); // custom rides first
    expect(s3[0].id).toBe(PROVIDER_CUSTOM);
    expect(s3[1].id).toBe("backblaze-b2"); // the launch order, stable
    // A lane with no presets still gets the custom entry — custom-only UI.
    const tus = providersForAdapter("tus");
    expect(tus.length).toBe(1);
    expect(tus[0].id).toBe(PROVIDER_CUSTOM);
  });
});

describe("buildEndpoint — the derivation math", () => {
  it("fills the region slot and appends the bucket", () => {
    const url = buildEndpoint(B2(), { region: "us-west-004", bucket: "my-bkt" });
    expect(url).toBe("https://s3.us-west-004.backblazeb2.com/my-bkt");
  });

  it("tracks a different region", () => {
    const url = buildEndpoint(B2(), { region: "eu-central-003", bucket: "my-bkt" });
    expect(url).toBe("https://s3.eu-central-003.backblazeb2.com/my-bkt");
  });

  it("fills text slots (the R2 account id)", () => {
    const r2 = providerById("cloudflare-r2");
    expect(r2).toBeDefined();
    const url = buildEndpoint(r2!, { account_id: "abc123", bucket: "my-bkt" });
    expect(url).toBe("https://abc123.r2.cloudflarestorage.com/my-bkt");
  });

  it("a slotless preset (Filebase) needs only the bucket", () => {
    const fb = providerById("filebase");
    expect(fb).toBeDefined();
    expect(buildEndpoint(fb!, { bucket: "my-bkt" })).toBe(
      "https://s3.filebase.com/my-bkt"
    );
  });

  it("is incomplete while any slot (or the bucket) is empty", () => {
    expect(canDerive(B2(), { region: "us-west-004", bucket: "" })).toBe(false);
    expect(canDerive(B2(), { region: "", bucket: "b" })).toBe(false);
    expect(canDerive(B2(), { region: "us-west-004", bucket: "b" })).toBe(true);
    expect(canDerive(providerById("filebase")!, { bucket: "" })).toBe(false);
  });

  it("refuses to derive with unknown slot values present", () => {
    // Extra values are ignored, never smuggled into the URL.
    const url = buildEndpoint(B2(), { region: "us-west-004", bucket: "b", evil: "x" });
    expect(url).not.toContain("evil");
    expect(url).not.toContain("x");
  });
});
