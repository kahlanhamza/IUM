// RED at this point: src/logic.ts does not exist and vitest is not wired.
import { describe, expect, it } from "vitest";
import {
  badgeFor,
  canCancel,
  canPause,
  canResume,
  confirmedTotal,
  friendlyAdapter,
  hostOf,
  percentDone,
  protocolOptions,
  recentEndpoints,
  speedBetween,
} from "./logic";

describe("honest badge mapping (#16, #65 Lane 1 structured badge)", () => {
  it("maps the engine's capability badge verbatim, never recomputed", () => {
    expect(badgeFor("resumable")).toEqual({ kind: "resumable", label: "Resumable" });
    expect(badgeFor("one_shot")).toEqual({ kind: "one_shot", label: "One-shot" });
  });
  it("refuses to invent a badge for an unknown capability", () => {
    expect(badgeFor("warp_drive" as never)).toEqual({ kind: "unknown", label: "unknown" });
  });
});

describe("adapter display names (#65 Lane 1)", () => {
  it("maps known registry names to friendly labels — values stay registry names", () => {
    expect(friendlyAdapter("http_oneshot")).toBe("HTTP one-shot");
    expect(friendlyAdapter("tus")).toBe("TUS 1.0.0");
    expect(friendlyAdapter("s3_multipart")).toBe("S3 multipart");
    expect(friendlyAdapter("feishu_drive")).toBe("Feishu Drive");
    expect(friendlyAdapter("terabox")).toBe("TeraBox");
    expect(friendlyAdapter("mega")).toBe("MEGA");
    expect(friendlyAdapter("webdav")).toBe("WebDAV");
    expect(friendlyAdapter("google_drive")).toBe("Google Drive");
    expect(friendlyAdapter("youtube")).toBe("YouTube");
  });
  it("an unknown registry name passes through untouched — the UI reflects the engine", () => {
    expect(friendlyAdapter("future_adapter")).toBe("future_adapter");
  });
});

describe("recent destinations (#65 Lane 1 zero-touch)", () => {
  it("most recent first, deduped, capped", () => {
    const jobs = [
      { endpoint: "http://a:1/files" },
      { endpoint: "http://b:2/files" },
      { endpoint: "http://a:1/files" },
      { endpoint: "" },
      { endpoint: "http://c:3/files" },
    ];
    expect(recentEndpoints(jobs as never, 3)).toEqual([
      "http://c:3/files",
      "http://a:1/files",
      "http://b:2/files",
    ]);
  });
  it("no jobs, no recents", () => {
    expect(recentEndpoints([], 8)).toEqual([]);
  });
});

describe("engine-mirroring action rules (#16)", () => {
  it("pause is enabled exactly while the job is non-terminal", () => {
    for (const s of ["created", "probing", "running", "paused", "finishing"])
      expect(canPause(s), s).toBe(true);
    for (const s of ["done", "failed", "canceled", "unresumable"])
      expect(canPause(s), s).toBe(false);
  });
  it("resume is enabled only from paused", () => {
    expect(canResume("paused")).toBe(true);
    for (const s of ["created", "running", "done", "canceled"])
      expect(canResume(s), s).toBe(false);
  });
  it("cancel is enabled before Finishing and never after", () => {
    for (const s of ["created", "probing", "running", "paused"])
      expect(canCancel(s), s).toBe(true);
    for (const s of ["finishing", "done", "failed", "canceled", "unresumable"])
      expect(canCancel(s), s).toBe(false);
  });
});

describe("progress math over journal snapshots (#16)", () => {
  it("sums only what the server confirmed", () => {
    const chunks = [
      { bytes_confirmed: 100 },
      { bytes_confirmed: 50 },
      { bytes_confirmed: 0 },
    ];
    expect(confirmedTotal(chunks as never)).toBe(150);
  });
  it("percent: confirmed/total; 0-byte jobs are 100% only when done", () => {
    expect(percentDone(150, 300, "running")).toBe(50);
    expect(percentDone(0, 0, "done")).toBe(100);
    expect(percentDone(0, 0, "created")).toBe(0);
  });
  it("speed is display math on two journal snapshots", () => {
    expect(speedBetween(100, 0, 1500, 500)).toBe(100); // 100 B in 1 s
    expect(speedBetween(500, 0, 1000, 500)).toBe(1000); // 500 B in 0.5 s
    expect(speedBetween(100, 0, 100, 500)).toBe(0); // no delta, no lie
  });
  it("host extraction keeps the destination readable", () => {
    expect(hostOf("https://bucket.example.com/s3/prefix")).toBe("bucket.example.com");
    expect(hostOf("http://localhost:1080/files")).toBe("localhost:1080");
    expect(hostOf("not a url")).toBe("not a url");
  });
});

describe("protocolOptions (#119): the full capability surface", () => {
  const SCHEMA = [
    { adapter: "s3_multipart", label: "S3 multipart" },
    { adapter: "mega", label: "MEGA" },
    { adapter: "feishu", label: "Feishu Drive" },
    { adapter: "dropbox", label: "Dropbox" },
    { adapter: "onedrive", label: "OneDrive" },
  ];

  it("registered lanes come first, labels unchanged, values are the registry names", () => {
    const out = protocolOptions(["http_oneshot", "tus"], SCHEMA);
    expect(out.slice(0, 2).map((o) => o.value)).toEqual(["http_oneshot", "tus"]);
    expect(out[0].label).toBe("HTTP one-shot");
    expect(out[0].connected).toBe(true);
  });

  it("supported-but-unregistered lanes follow, honestly suffixed", () => {
    const out = protocolOptions(["http_oneshot", "tus"], SCHEMA);
    const rest = out.slice(2);
    expect(rest.map((o) => o.value)).toEqual([
      "s3_multipart",
      "mega",
      "feishu",
      "dropbox",
      "onedrive",
    ]);
    expect(rest.every((o) => !o.connected)).toBe(true);
    expect(rest[0].label).toBe("S3 multipart — not connected yet");
    expect(rest[1].label).toBe("MEGA — not connected yet");
  });

  it("a registered lane never doubles as an unregistered one (the feishu_drive name seat)", () => {
    // The engine registers FeishuAdapter under `feishu_drive`; the vault
    // schema lists it as `feishu`. Same product lane: ONE option, connected.
    const out = protocolOptions(["feishu_drive"], SCHEMA);
    expect(out.map((o) => o.value)).toEqual([
      "feishu_drive",
      "s3_multipart",
      "mega",
      "dropbox",
      "onedrive",
    ]);
    expect(out[0]).toEqual({ value: "feishu_drive", label: "Feishu Drive", connected: true });
  });

  it("the empty registry still shows the whole surface — the fresh-machine fix", () => {
    const out = protocolOptions([], SCHEMA);
    expect(out).toHaveLength(SCHEMA.length);
    expect(out.every((o) => !o.connected)).toBe(true);
  });
});
