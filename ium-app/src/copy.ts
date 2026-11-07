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
export type { CopyRule };

/**
 * The rules, exported for the brand gate (#73): the copy surface is
 * enumerated, so the voice rules can be asserted against every sentence.
 */
export const RULES: CopyRule[] = [
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

/** The fall-back sentence pair, exported for the brand gate (#73). */
export const DEFAULT_COPY = {
  title: DEFAULT_TITLE,
  hint: "The engine's own words are below — they help report the problem.",
} as const;

/**
 * The app-level sentences (the #73 copy surface): the created toast and the
 * empty state, moved here so every user-visible sentence lives in ONE table
 * and the brand gate can assert the voice over all of them.
 */
export const APP_COPY = {
  created: "Upload created — the engine is driving it.",
  emptyTitle: "No uploads yet",
  emptyHint:
    "Drop a file above, choose a destination, and press Upload. Progress, pause, resume and cancel are all journaled — a crash never loses ground.",
  // #80: the silent startup check found a newer release. {version} is
  // replaced at the call site; the install control lives in Settings → About.
  updateAvailable:
    "IUM {version} is available — Settings → About has the install control.",
  // #81: the credentials lanes re-register live — the restart sentences are
  // gone; these state what actually happens now.
  authSavedLive: "Credentials saved — the transfer lane re-registered, no restart needed.",
  authRemovedLive: "Credentials removed — the transfer lane is gone from the registry.",
  // #81: where the secret VALUES actually live on this machine (the honesty
  // marker the footer prints; the fallback keeps the 0600 vault file).
  secretsKeychain:
    "Secrets are stored in your OS keychain — the vault file carries non-secret fields only.",
  secretsFileFallback:
    "Your OS keychain service is unavailable — secrets are stored in the 0600 vault file instead.",
  authFooterEnv: "A real environment variable always wins over a saved value.",
  authFooterLive: "Saving re-registers the transfer lane immediately — no restart.",
  // #82: the one-click connect surface — the per-lane state sentences and
  // the connect sheet's outcomes. Calm sentences, no exclamation marks.
  connectStateConnected: "Connected.",
  connectStatePartial: "Almost there — some required details are missing.",
  connectStateEmpty: "Not set up yet.",
  connectDone: "Connected — the transfer lane is live in the registry, no restart needed.",
  oauthWaiting:
    "Your browser opened — sign in to the platform there. IUM finishes by itself when you do.",
  oauthDone: "Connected — the token lives in your OS keychain.",
} as const;

/**
 * The telemetry Privacy surface (#66 Lane 7, #75): every sentence the
 * Settings → Privacy card renders, enumerated here so the brand gate's
 * voice rules cover it like the rest of the copy table. The stance itself
 * is the charter (docs/11_Telemetry_Stance.md) — these sentences must be
 * able to state it honestly: collection is opt-out, the record is local,
 * this build uploads nothing.
 */
export const TELEMETRY_COPY = {
  stance:
    "IUM collects anonymous diagnostics to keep uploads honest — the record stays on your machine, and one switch turns it off.",
  switchLabel: "Collect anonymous diagnostics",
  collectJob:
    "For each finished upload: the transfer method, how it ended, and the size and duration as wide buckets — never the file name, never the destination.",
  collectCrash:
    "When IUM itself fails: the panic message and where in IUM's own code it fired — nothing else about your session.",
  neverLine:
    "IUM never collects file names, folder paths, destinations, credentials, account ids or file contents.",
  bufferNote:
    "Events live in a local file you can open and clear. This build uploads nothing anywhere.",
  consentOnToast: "Diagnostics collection is on — events stay in the local file.",
  consentOffToast: "Diagnostics collection is off — nothing more is recorded.",
  clearedToast: "The local diagnostics file is cleared.",
} as const;

/**
 * The exact field set a telemetry event may carry — the TS mirror of the
 * charter's collected-fields table (docs/11_Telemetry_Stance.md). The
 * charter gate asserts table == this list; the Rust harness asserts the
 * same table against the writers. Neither side moves without the other.
 */
export const TELEMETRY_FIELDS = [
  "v",
  "kind",
  "ts",
  "adapter",
  "state",
  "size_bucket",
  "duration_bucket",
  "message",
  "source",
  "line",
  "column",
] as const;

export function humanizeError(raw: string): ErrorCopy {
  const lowered = raw.toLowerCase();
  for (const rule of RULES) {
    if (rule.patterns.some((p) => p.test(lowered))) {
      return { title: rule.title, hint: rule.hint, raw };
    }
  }
  return { title: DEFAULT_COPY.title, hint: DEFAULT_COPY.hint, raw };
}
