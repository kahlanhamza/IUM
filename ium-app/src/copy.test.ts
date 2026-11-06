/**
 * The error-copy contracts (#66 Lane 3, #70): every user-visible failure
 * sentence comes from THIS table, matched against the real engine Display
 * strings. The raw truth is never replaced — the UI shows the humanized
 * sentence and keeps `the engine said: <raw>` in view.
 */
import { describe, expect, it } from "vitest";
import { humanizeError } from "./copy";

describe("humanizeError — the adapter-error family (real Display prefixes)", () => {
  it("auth required → the credentials sentence + Settings hint", () => {
    const c = humanizeError("adapter error: auth required: tenant token rejected");
    expect(c.title).toMatch(/rejected the credentials/i);
    expect(c.hint).toMatch(/Settings/i);
  });

  it("expired session → re-enter credentials, resume is safe", () => {
    const c = humanizeError("adapter error: expired session: 1061021");
    expect(c.title).toMatch(/expired/i);
    expect(c.hint).toMatch(/resumes from where it stopped/i);
  });

  it("session lost → resume from the last confirmed byte", () => {
    const c = humanizeError("adapter error: session lost: serve died");
    expect(c.hint).toMatch(/last confirmed byte/i);
  });

  it("throttled and 429 land on the same backoff copy", () => {
    expect(humanizeError("adapter error: throttled: slow down").hint).toMatch(/backs off/i);
    expect(humanizeError("HTTP 429 too many requests").hint).toMatch(/backs off/i);
  });

  it("corruption → the verify-before-declare promise", () => {
    const c = humanizeError("adapter error: corruption: 1062009");
    expect(c.title).toMatch(/corrupt/i);
    expect(c.hint).toMatch(/bytes/i);
  });

  it("unresumable → the honest one-shot explanation", () => {
    const c = humanizeError("adapter error: unresumable: one-shot family");
    expect(c.hint).toMatch(/protocol's limit, not a bug/i);
  });

  it("busy → the engine is told to wait, not the user", () => {
    const c = humanizeError("adapter error: busy: 1061045");
    expect(c.hint).toMatch(/automatically/i);
  });
});

describe("humanizeError — the engine/journal family", () => {
  it("unregistered adapter points at Settings", () => {
    const c = humanizeError("adapter 'mega' is not registered");
    expect(c.hint).toMatch(/Settings/i);
  });

  it("wrong-state explains the legal transitions", () => {
    const c = humanizeError("job abc is in state done; operation requires running");
    expect(c.hint).toMatch(/legal transitions/i);
  });

  it("journal errors promise the disk is safe", () => {
    expect(humanizeError("journal error: sqlite: disk I/O").hint).toMatch(/safe on disk/i);
  });

  it("a missing job defers to the list", () => {
    const c = humanizeError("job abc not found");
    expect(c.hint).toMatch(/journal's truth/i);
  });
});

describe("humanizeError — the network family (reqwest strings)", () => {
  it("dns failure → check the endpoint URL", () => {
    const c = humanizeError("adapter error: unknown: error sending request: dns error");
    expect(c.title).toMatch(/resolve/i);
    expect(c.hint).toMatch(/typos/i);
  });

  it("connection refused → reachability copy", () => {
    const c = humanizeError("error sending request: connection refused");
    expect(c.title).toMatch(/reach the destination/i);
  });

  it("timeout → the eviction promise", () => {
    expect(humanizeError("operation timed out").hint).toMatch(/evicts slow connections/i);
  });
});

describe("humanizeError — the honest default", () => {
  it("unknown errors never invent a cause and carry the raw for display", () => {
    const c = humanizeError("some entirely new failure mode");
    expect(c.title).toMatch(/went wrong/i);
    expect(c.raw).toBe("some entirely new failure mode");
  });

  it("the engine-unreachable rule wins over the generic network rule", () => {
    const c = humanizeError("engine unreachable: connection refused");
    expect(c.title).toMatch(/upload engine/i);
    expect(c.hint).toMatch(/restarting the app/i);
  });
});
