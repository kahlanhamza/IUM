/**
 * The share-link moment in the inspector (the destination lane, Tier 2).
 * The gate (`canShareJob`) is pinned in share.test.ts; this pins the
 * COMPONENT: the button exists ONLY for a finished s3 job, the click mints
 * + copies the link, the expiry sentence is spelled out, and the shell's
 * refusal lands as the honest failed note — never a fake success.
 */
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

const shareLink = vi.fn();

vi.mock("../api", () => ({
  api: {
    shareLink: (...args: [string]) => shareLink(...args),
  },
}));

import { InspectorDrawer } from "./InspectorDrawer";
import type { JobSnapshot } from "../api";

function snap(adapter: string, state: string): JobSnapshot {
  return {
    job: {
      id: "job-1",
      source_path: "/videos/launch.mp4",
      source_size: 12 * 1024 * 1024,
      adapter,
      endpoint: "https://s3.us-west-004.backblazeb2.com/ium-bucket/launch.mp4",
      state,
      capability_badge: "resumable",
      created_at: 0,
      updated_at: 0,
    },
    chunks: [],
    events: [],
  };
}

function mount(snapshot: JobSnapshot | null) {
  render(<InspectorDrawer snapshot={snapshot} onClose={() => {}} />);
}

afterEach(() => {
  cleanup();
  shareLink.mockReset();
});

describe("the inspector's share moment", () => {
  it("a finished s3 job carries the copy-share button and mints the link", async () => {
    const url = "https://s3.us-west-004.backblazeb2.com/ium-bucket/launch.mp4?X-Amz-Signature=abc";
    shareLink.mockResolvedValue({ url, expires_at: 1_800_000_000 });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    mount(snap("s3_multipart", "done"));
    const button = screen.getByRole("button", { name: /Copy share link/i });
    fireEvent.click(button);
    expect(screen.getByRole("button", { name: /Signing…/i })).toBeDefined();
    await waitFor(() => expect(shareLink).toHaveBeenCalledWith("job-1"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(url));
    expect(screen.getByText(/opens for 24 hours, then expires/i)).toBeDefined();
  });

  it("a mint failure is honest — never a fake copied", async () => {
    shareLink.mockRejectedValue("No S3 keys saved — Settings → S3 multipart first.");
    Object.assign(navigator, { clipboard: { writeText: vi.fn() } });

    mount(snap("s3_multipart", "done"));
    fireEvent.click(screen.getByRole("button", { name: /Copy share link/i }));
    await waitFor(() => expect(screen.getByText(/could not be signed/i)).toBeDefined());
    expect(navigator.clipboard.writeText).not.toHaveBeenCalled();
  });

  it("nothing unfinished and no other lane ever sees the button", () => {
    for (const [adapter, state] of [
      ["s3_multipart", "running"],
      ["s3_multipart", "failed"],
      ["s3_multipart", "canceled"],
      ["tus", "done"],
      ["http_oneshot", "done"],
      ["webdav", "done"],
    ] as const) {
      const { unmount } = render(
        <InspectorDrawer snapshot={snap(adapter, state)} onClose={() => {}} />
      );
      expect(screen.queryByRole("button", { name: /Copy share link/i })).toBeNull();
      unmount();
      cleanup();
    }
  });
});
