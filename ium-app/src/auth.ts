/**
 * The test-connection state machine (#81): the owner's bar is auth "should
 * not be painful, it should be so easy". A Test button that answers BEFORE
 * the first upload — speak sentences, never codes; failures route through
 * the copy table (humanizeError) so "the destination rejected the
 * credentials" and the raw engine words render exactly like upload errors
 * do. No Tauri imports — testable in plain vitest (the #69 pattern).
 */

import { humanizeError } from "./copy";

export type ProbeState = "idle" | "testing" | "ok" | "fail" | "unchecked";

/** The probe result the shell returns (#81). */
export interface ProbeResult {
  /** false = the family has no no-job credential check — the honest
   * "verified on the first upload" answer, never a fake green check. */
  checked: boolean;
  ok: boolean;
  /** ok: the acceptance sentence. fail + final: the honest human sentence
   * (a missing required field). fail otherwise: the RAW engine string,
   * for the copy table. unchecked: the honest not-checked sentence. */
  sentence: string;
  /** true = `sentence` is already final copy; false = route through the
   * copy table and keep the raw visible. */
  final: boolean;
}

/** The acceptance sentence the shell sends on a successful check. */
export const PROBE_OK = "The server accepted the credentials.";

/** The honest not-checked sentence the shell sends for families without a
 * no-job check (the s3 family: its endpoint is per-job, its probe creates
 * sessions — a Test button must never orphan a real upload). */
export const PROBE_UNCHECKED =
  "Not checked here — this lane verifies itself on the first upload.";

/**
 * The Test control's copy per state. `idle` is the button label; the
 * result states render the sentence the shell sent (with the honest
 * defaults when the IPC could not answer).
 */
export function probeCopy(state: ProbeState, result?: ProbeResult): string {
  switch (state) {
    case "idle":
      return "Test connection";
    case "testing":
      return "Testing…";
    case "ok":
      return result?.sentence || PROBE_OK;
    case "unchecked":
      return result?.sentence || PROBE_UNCHECKED;
    case "fail":
      if (!result) return "The test could not run.";
      return result.final ? result.sentence : humanizeError(result.sentence).title;
  }
}

/**
 * The fail rendering beyond the title: the table's hint plus the raw truth
 * — the UI never hides the engine's own words (the #70 contract).
 */
export function probeFailCopy(result: ProbeResult) {
  return humanizeError(result.sentence);
}
