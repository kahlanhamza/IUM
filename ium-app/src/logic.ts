/**
 * Pure presentation logic for the jobs table (#16, spec §8).
 *
 * Rule (spec §2.3): the UI never computes TRANSFER truth — every number here
 * is derived FROM journal snapshots for display only. The action rules mirror
 * the engine's state machine (`ium-core/src/state.rs`) verbatim: the buttons
 * can only ask the engine for what the engine would accept.
 */

export type CapabilityBadge = "resumable" | "one_shot";
export type JobStateStr =
  | "created"
  | "probing"
  | "running"
  | "paused"
  | "finishing"
  | "done"
  | "unresumable"
  | "failed"
  | "canceled";

const TERMINAL: readonly string[] = ["done", "failed", "canceled", "unresumable"];
const CANCELABLE: readonly string[] = ["created", "probing", "running", "paused"];

/**
 * Honest badges: rendered verbatim from the engine, unknown stays unknown.
 * Structured view (#65 Lane 1): the chip's color/icon live in the component
 * layer; the label text stays the engine's own vocabulary.
 */
export interface BadgeView {
  kind: "resumable" | "one_shot" | "unknown";
  label: string;
}

export function badgeFor(capability: string): BadgeView {
  if (capability === "resumable") return { kind: "resumable", label: "Resumable" };
  if (capability === "one_shot") return { kind: "one_shot", label: "One-shot" };
  return { kind: "unknown", label: "unknown" };
}

/**
 * Friendly adapter display names (#65 Lane 1): presentation-only mapping —
 * the SELECT's option values stay the engine registry names (the e2e seam
 * sets `.value` directly), an unknown registry name passes through untouched
 * because the UI reflects the engine, it never invents lanes.
 */
/**
 * The friendly labels, exported for the brand gate (#73): product surfaces
 * are friendly, engine seams stay technical — the table is the contract.
 */
export const ADAPTER_LABELS: Record<string, string> = {
  http_oneshot: "HTTP one-shot",
  tus: "TUS resumable",
  s3_multipart: "S3 multipart",
  feishu_drive: "Feishu Drive",
  terabox: "TeraBox",
  mega: "MEGA",
};

export function friendlyAdapter(name: string): string {
  return ADAPTER_LABELS[name] ?? name;
}

/**
 * Recent destinations for the endpoint datalist (#65 Lane 1 zero-touch):
 * newest job first, deduped, empty strings dropped, capped. Presentation
 * math over journal records — no transfer truth.
 */
export function recentEndpoints(jobs: { endpoint: string }[], max = 8): string[] {
  const seen: string[] = [];
  for (let i = jobs.length - 1; i >= 0 && seen.length < max; i--) {
    const ep = jobs[i].endpoint;
    if (ep && !seen.includes(ep)) seen.push(ep);
  }
  return seen;
}

export function isTerminal(state: string): boolean {
  return TERMINAL.includes(state);
}

/** The engine accepts pause from any non-terminal state. */
export function canPause(state: string): boolean {
  return !isTerminal(state);
}

/** Only a paused job may resume. */
export function canResume(state: string): boolean {
  return state === "paused";
}

/** Cancel before Finishing; verify in flight completes honestly as Done. */
export function canCancel(state: string): boolean {
  return CANCELABLE.includes(state);
}

export interface ChunkLike {
  bytes_confirmed: number;
}

/** Σ server-confirmed bytes across the plan — nothing else counts. */
export function confirmedTotal(chunks: ChunkLike[]): number {
  return chunks.reduce((sum, c) => sum + c.bytes_confirmed, 0);
}

/**
 * Display percent. A 0-byte upload is 100% only when the engine says Done —
 * an in-flight 0-byte job has nothing to show yet.
 */
export function percentDone(confirmed: number, total: number, state: string): number {
  if (total === 0) return state === "done" ? 100 : 0;
  return Math.min(100, Math.floor((confirmed / total) * 100));
}

/**
 * Live speed as Δconfirmed/Δt between two snapshots of the same job
 * (bytes/sec). Presentation math on journal numbers; 0 when nothing moved.
 */
export function speedBetween(
  confirmedNow: number,
  confirmedBefore: number,
  msNow: number,
  msBefore: number,
): number {
  const bytes = confirmedNow - confirmedBefore;
  const seconds = (msNow - msBefore) / 1000;
  if (bytes <= 0 || seconds <= 0) return 0;
  return Math.round(bytes / seconds);
}

/** Host of an endpoint URL for the destination column; raw text otherwise. */
export function hostOf(endpoint: string): string {
  try {
    return new URL(endpoint).host;
  } catch {
    return endpoint;
  }
}

export function humanBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let v = n;
  let u = "B";
  for (const unit of units) {
    if (v < 1024) break;
    v /= 1024;
    u = unit;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${u}`;
}
