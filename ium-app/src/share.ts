/**
 * The shareable-link gate (the destination lane, Tier 2). The rule is
 * deliberately narrow: only a FINISHED s3_multipart job can produce a
 * link, and only because the adapter's own verify (ADR-010) already said
 * the destination holds the bytes. Other lanes have no defined link
 * semantics yet — they never fake one. The link itself is presigned by
 * the shell (`share_link` command) from the vault's keys; nothing is ever
 * auto-published and no link exists until the user asks.
 */
import type { JobRecord } from "./api";

/** `done` is the engine's honest-complete state (the verify gate passed). */
export function canShareJob(job: Pick<JobRecord, "adapter" | "state">): boolean {
  return job.adapter === "s3_multipart" && job.state === "done";
}

/** v1 pins one honest TTL: 24 hours (within the S3 7-day maximum). */
export function shareTtlSeconds(): number {
  return 24 * 3600;
}
