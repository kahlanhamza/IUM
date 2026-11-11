/**
 * The shareable link gate (the destination lane, Tier 2). RED artifact:
 * `./share` does not exist at the pre-implementation HEAD. The rule: a
 * finished s3_multipart job can hand the user a link — nothing else, and
 * only `done` (the adapter's own verify said the destination holds the
 * bytes; ADR-010). Other lanes' links are not defined yet and NEVER fake.
 */
import { describe, expect, it } from "vitest";
import { canShareJob, shareTtlSeconds } from "./share";

const job = (adapter: string, state: string) => ({ adapter, state });

describe("canShareJob", () => {
  it("a finished S3 job can share", () => {
    expect(canShareJob(job("s3_multipart", "done"))).toBe(true);
  });

  it("nothing half-finished shares", () => {
    for (const state of ["created", "probing", "running", "paused", "finishing", "failed", "canceled"]) {
      expect(canShareJob(job("s3_multipart", state)), state).toBe(false);
    }
  });

  it("no other lane shares yet (honest, never fake)", () => {
    for (const adapter of ["tus", "http_oneshot", "webdav", "google_drive", "mega", "terabox", "feishu", "youtube", "rclone_bridge"]) {
      expect(canShareJob(job(adapter, "done")), adapter).toBe(false);
    }
  });
});

describe("shareTtlSeconds", () => {
  it("v1 pins a 24-hour link", () => {
    expect(shareTtlSeconds()).toBe(24 * 3600);
  });
});
