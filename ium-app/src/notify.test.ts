// @vitest-environment jsdom
//
// The terminal-notification pins (#93): the OS notification surface for the
// two terminal job states. The copy matrix is the design face — every word
// the OS shows is pinned here. The transition rule is pure (notifyDraftFor):
// prev/next + options → a draft or nothing, so the listener path, the poll
// path, and a manual refresh all take the same honest path.
import { afterEach, describe, expect, it } from "vitest";
import { NOTIFICATIONS_COPY, humanizeError } from "./copy";
import {
  basenameOf,
  loadNotifyPref,
  NOTIFY_STORAGE_KEY,
  notifyDraftFor,
  type NotifyDraft,
  type NotifyOptions,
} from "./notify";

function rec(state: string) {
  return {
    id: "j1",
    source_path: "/home/user/exports/clip.mp4",
    source_size: 1234,
    adapter: "webdav",
    endpoint: "https://dest.example/dav",
    state,
    capability_badge: "resumable" as const,
    created_at: 1,
    updated_at: 2,
  };
}

const ON = { enabled: true, focused: false };

afterEach(() => {
  window.localStorage.clear();
});

describe("terminal notifications (#93)", () => {
  it("first sight of a job never notifies — boot re-listing must not spam", () => {
    expect(notifyDraftFor(undefined, rec("done"), ON)).toBeNull();
    expect(notifyDraftFor(undefined, rec("failed"), ON)).toBeNull();
    expect(notifyDraftFor(undefined, rec("running"), ON)).toBeNull();
  });

  it("running → done notifies once, in the copy table's words", () => {
    expect(notifyDraftFor("running", rec("done"), ON)).toEqual({
      title: NOTIFICATIONS_COPY.doneTitle,
      body: "clip.mp4 reached webdav.",
    } satisfies NotifyDraft);
  });

  it("the same terminal state re-emitting notifies nothing (idempotent)", () => {
    expect(notifyDraftFor("done", rec("done"), ON)).toBeNull();
    expect(notifyDraftFor("failed", rec("failed"), ON)).toBeNull();
  });

  it("paused → done notifies too — the resume that finished still ends", () => {
    expect(notifyDraftFor("paused", rec("done"), ON)?.title).toBe(
      NOTIFICATIONS_COPY.doneTitle,
    );
  });

  it("queued → done notifies", () => {
    expect(notifyDraftFor("queued", rec("done"), ON)?.title).toBe(
      NOTIFICATIONS_COPY.doneTitle,
    );
  });

  it("failed carries the copy-table sentence for the engine's raw words", () => {
    const raw = "adapter error: auth required";
    const draft = notifyDraftFor("running", rec("failed"), { ...ON, raw });
    expect(draft).not.toBeNull();
    expect(draft?.title).toBe(NOTIFICATIONS_COPY.failedTitle);
    expect(draft?.body).toBe(
      `clip.mp4: ${humanizeError(raw).title}.`,
    );
  });

  it("failed without a raw word never invents a cause (the #70 rule)", () => {
    expect(notifyDraftFor("running", rec("failed"), ON)?.body).toBe(
      "clip.mp4 failed.",
    );
  });

  it("the toggle off silences every terminal state", () => {
    const off: NotifyOptions = { ...ON, enabled: false };
    expect(notifyDraftFor("running", rec("done"), off)).toBeNull();
    expect(notifyDraftFor("running", rec("failed"), off)).toBeNull();
  });

  it("a focused window silences the OS surface — the in-app toast already spoke", () => {
    expect(notifyDraftFor("running", rec("done"), { ...ON, focused: true })).toBeNull();
    expect(notifyDraftFor("running", rec("failed"), { ...ON, focused: true })).toBeNull();
  });

  it("the basename, never the machine's path, reaches the OS surface", () => {
    expect(basenameOf("/home/user/exports/clip.mp4")).toBe("clip.mp4");
    expect(basenameOf("clip.mp4")).toBe("clip.mp4");
    expect(basenameOf("C:\\Users\\u\\clip.mp4")).toBe("clip.mp4");
  });

  it("the pref persists — the theme store's shape, localStorage-backed", () => {
    expect(loadNotifyPref()).toBe(true); // the default is ON
    window.localStorage.setItem("ium.notify.enabled", "0");
    expect(loadNotifyPref()).toBe(false);
    window.localStorage.setItem("ium.notify.enabled", "1");
    expect(loadNotifyPref()).toBe(true);
    expect(window.localStorage.getItem("ium.notify.enabled")).toBe("1");
    expect(NOTIFY_STORAGE_KEY).toBe("ium.notify.enabled");
  });
});
