/**
 * The error-copy table (#66 Lane 3, #70): engine error strings become human
 * sentences with an action hint. The table matches the REAL Display strings
 * the engine emits (adapter.rs kinds, EngineError, JournalError, the reqwest
 * family, Feishu loud codes inside `adapter error: ...`) — first match wins,
 * and the default never invents a cause: the raw string stays available for
 * the UI's "the engine said: …" line.
 */

export interface ErrorCopy {
  /** The sentence the user sees first. */
  title: string;
  /** What to do about it — empty when there is nothing to do. */
  hint: string;
  /** The engine's own words — the UI never hides them. */
  raw: string;
}

interface CopyRule {
  patterns: RegExp[];
  title: string;
  hint: string;
}

const RULES: CopyRule[] = [
  {
    // The engine lives INSIDE the app — a different "unreachable" than a
    // remote endpoint; keep it ahead of the generic network rule.
    patterns: [/engine unreachable/],
    title: "Can't reach the upload engine",
    hint: "The engine runs inside IUM — restarting the app usually clears this.",
  },
  {
    // 401/403 and the AdapterError::AuthRequired family.
    patterns: [/auth required/, /\b401\b/, /\b403\b/, /unauthorized/, /forbidden/],
    title: "The destination rejected the credentials",
    hint: "Check this protocol's credentials in Settings.",
  },
  {
    // Feishu 1061021 and the generic expired-session kind.
    patterns: [/expired session/, /session expired/, /1061021/],
    title: "The destination session expired",
    hint: "Re-enter the credentials in Settings — the upload resumes from where it stopped.",
  },
  {
    patterns: [/session lost/],
    title: "The transfer session was lost",
    hint: "Resume the job — it continues from the last confirmed byte.",
  },
  {
    patterns: [/throttled/, /\b429\b/, /too many requests/, /rate limit/],
    title: "The destination is limiting upload speed",
    hint: "The engine backs off automatically; try fewer connections if it repeats.",
  },
  {
    // Feishu 1062009 and the corruption kind — verify-before-declare's territory.
    patterns: [/corruption/, /1062009/, /checksum mismatch/],
    title: "The upload was corrupted in transit",
    hint: "The engine re-sends the affected parts — Done requires the bytes to match.",
  },
  {
    patterns: [/unresumable/],
    title: "This destination can't resume",
    hint: "One-shot destinations restart from zero after an interruption — the protocol's limit, not a bug.",
  },
  {
    // Feishu 1061045 and the busy kind: the engine waits, not the user.
    patterns: [/busy/, /1061045/],
    title: "The destination is busy",
    hint: "The engine waits and retries automatically — no action needed.",
  },
  {
    patterns: [/is not registered/],
    title: "This transfer method isn't available",
    hint: "It appears in the list only when its credentials are set — check Settings.",
  },
  {
    patterns: [/is in state/, /illegal state transition/],
    title: "That action isn't allowed in the job's current state",
    hint: "Pause, resume and cancel follow the engine's legal transitions.",
  },
  {
    patterns: [/journal error/, /sqlite/],
    title: "The local journal reported a problem",
    hint: "Your data is safe on disk — restarting IUM re-opens the journal.",
  },
  {
    patterns: [/not found in journal/, /job \S+ not found/],
    title: "That upload no longer exists",
    hint: "It may have been removed — the list shows the journal's truth.",
  },
  {
    patterns: [/dns error/, /no such host/, /name or service not known/, /failed to lookup/],
    title: "The destination address doesn't resolve",
    hint: "Check the endpoint URL for typos.",
  },
  {
    patterns: [/connection refused/, /error sending request/, /network changed/, /unreachable/],
    title: "Can't reach the destination",
    hint: "Check the endpoint URL and your connection.",
  },
  {
    patterns: [/timed out/, /timeout/],
    title: "The destination stopped responding",
    hint: "The engine evicts slow connections automatically. Check your network if it repeats.",
  },
  {
    patterns: [/certificate/, /tls/, /ssl/],
    title: "The destination's security certificate failed",
    hint: "If you trust this endpoint, its certificate may have expired or be self-signed.",
  },
];

const DEFAULT_TITLE = "Something went wrong";

export function humanizeError(raw: string): ErrorCopy {
  const lowered = raw.toLowerCase();
  for (const rule of RULES) {
    if (rule.patterns.some((p) => p.test(lowered))) {
      return { title: rule.title, hint: rule.hint, raw };
    }
  }
  return { title: DEFAULT_TITLE, hint: "The engine's own words are below — they help report the problem.", raw };
}
