import { describe, expect, it } from "vitest";
import { APP_COPY } from "./copy";
import { probeCopy, probeFailCopy, PROBE_OK, PROBE_UNCHECKED, type ProbeResult } from "./auth";

// The painless-auth surface (#81): the owner's bar is "it should not be
// painful, it should be so easy". Three restart sentences die and one
// honest machine takes their place: a Test connection control whose fail
// path speaks the copy table's rule — "the destination rejected the
// credentials", never "Something happened" alone, raw truth preserved.

describe("the test-connection control (#81)", () => {
  it("idle and testing speak the two control states", () => {
    expect(probeCopy("idle")).toMatch(/Test connection/);
    expect(probeCopy("testing")).toMatch(/Testing/);
  });

  it("ok speaks the acceptance sentence", () => {
    expect(probeCopy("ok")).toBe(PROBE_OK);
    const sent: ProbeResult = { checked: true, ok: true, sentence: PROBE_OK, final: true };
    expect(probeCopy("ok", sent)).toMatch(/accepted the credentials/i);
  });

  it("unchecked is the honest verify-on-first-upload answer — never a fake green check", () => {
    expect(probeCopy("unchecked")).toBe(PROBE_UNCHECKED);
    expect(probeCopy("unchecked", { checked: false, ok: true, sentence: PROBE_UNCHECKED, final: true })).toMatch(
      /first upload/i,
    );
  });

  it("fail speaks through the copy table — rejected credentials, raw preserved", () => {
    const raw = "adapter error: auth required: tenant token rejected";
    const r: ProbeResult = { checked: true, ok: false, sentence: raw, final: false };
    expect(probeCopy("fail", r)).toMatch(/rejected the credentials/i);
    const full = probeFailCopy(r);
    expect(full.raw).toBe(raw);
    expect(full.hint).toMatch(/Settings/i);
  });

  it("a final fail sentence (a missing field) renders as-is, no table detour", () => {
    const r: ProbeResult = {
      checked: true,
      ok: false,
      sentence: "The ndus cookie is required for a test — paste it first.",
      final: true,
    };
    expect(probeCopy("fail", r)).toMatch(/ndus cookie is required/);
  });

  it("fail without a result still speaks a full sentence", () => {
    expect(probeCopy("fail")).toMatch(/\S/);
  });
});

describe("the live-apply copy (#81)", () => {
  it("the restart sentences are gone from the enumerated copy", () => {
    const all = Object.values(APP_COPY).join(" ");
    expect(all).not.toMatch(/next time IUM starts/i);
    expect(all).not.toMatch(/Credentials are read when IUM starts/i);
  });

  it("save and removal say the lane re-registered live", () => {
    expect(APP_COPY.authSavedLive).toMatch(/saved/i);
    expect(APP_COPY.authSavedLive).toMatch(/re-registered/i);
    // The OLD semantics die: "the next time IUM starts" / "takes effect
    // after restart" must not return in any shape ("no restart needed"
    // is the honest negation and is fine).
    expect(APP_COPY.authSavedLive).not.toMatch(/next time|takes effect after/i);
    expect(APP_COPY.authRemovedLive).toMatch(/removed/i);
  });

  it("the footer says where secrets live — keychain first, fallback honest", () => {
    expect(APP_COPY.secretsKeychain).toMatch(/OS keychain/i);
    expect(APP_COPY.secretsFileFallback).toMatch(/unavailable/i);
    expect(APP_COPY.authFooterLive).toMatch(/immediately|no restart/i);
  });
});
