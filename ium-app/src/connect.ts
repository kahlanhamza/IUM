/**
 * The connect surface's pure math (#82): what the one-click connect row
 * says about each lane, and which platforms offer an obvious browser
 * sign-in. The owner's own framing is the spec: S3 "might be any service"
 * — generic protocols keep their focused credential sheet; the obvious
 * OAuth platforms get the Google-style popup (the browser opens the
 * provider's real page, the user signs in as normal, IUM finishes by
 * itself when they do). DRIVE joined when the owner lifted the shelving
 * (#84 → #59); YouTube rides its own native flow (#85).
 * No Tauri imports — plain vitest (the #69 pattern).
 */
import type { FieldRow } from "./settings";

export type ConnectionState = "connected" | "partial" | "empty";

/** Saved required fields = the lane exists in the registry. A draft is a
 *  typed-but-unsaved value — honest chips read SAVED state only. */
export function connectionState(rows: FieldRow[]): ConnectionState {
  const required = rows.filter((r) => r.required);
  const saved = required.filter((r) => r.present).length;
  if (required.length === 0 || saved === 0) return "empty";
  if (saved === required.length) return "connected";
  return "partial";
}

export interface OAuthBackend {
  id: string;
  label: string;
  /** The one sign-in sentence the picker shows under the platform. */
  note: string;
}

/** The obvious browser-sign-in platforms (the rclone authorize family).
 *  An ALLOWLIST, not a free-form field: the shell refuses any id outside
 *  it before a spawned command line can ever see it (the injection
 *  contract). Drive joined when the owner lifted the shelving (#84);
 *  YouTube stays out — rclone has no such backend (#85's own flow). */
export const OAUTH_BACKENDS: OAuthBackend[] = [
  { id: "pcloud", label: "pCloud", note: "Sign in with your pCloud account in the browser." },
  { id: "onedrive", label: "OneDrive", note: "Sign in with your Microsoft account in the browser." },
  { id: "box", label: "Box", note: "Sign in with your Box account in the browser." },
  { id: "drive", label: "Google Drive", note: "Sign in with your Google account in the browser." },
];

/** The allowlist check both ends enforce — the UI for the picker, the
 *  shell before spawning (one contract, two readers). */
export function isOAuthBackend(id: string): boolean {
  return OAUTH_BACKENDS.some((b) => b.id === id);
}
