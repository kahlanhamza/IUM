/**
 * Typed IPC contract — the upload twin of Motrix's MBP1 bridge lesson:
 * one stable, versioned surface between UI and engine (spec §2.1).
 * Mirrors the #[tauri::command]s in src-tauri/src/lib.rs.
 */
import { invoke } from "@tauri-apps/api/core";

/** The masked credentials view (#69): secrets never carry values. */
export interface FieldView {
  label: string;
  secret: boolean;
  required: boolean;
  present: boolean;
  value: string | null;
  /** No input renders (#84): the OAuth lanes' token row is a saved-state
   *  chip — the sign-in writes it inside the process, the UI never sees a
   *  token byte, its PRESENCE is the state. */
  hidden: boolean;
}

export type AdapterViews = Record<string, Record<string, FieldView>>;

export interface AdapterSchema {
  adapter: string;
  label: string;
  note: string;
  fields: { name: string; label: string; secret: boolean; required: boolean }[];
}

export interface JobSpec {
  source_path: string;
  source_size: number;
  adapter: string;
  endpoint: string;
}

export interface JobRecord {
  id: string;
  source_path: string;
  source_size: number;
  adapter: string;
  endpoint: string;
  state: string;
  capability_badge: "resumable" | "one_shot";
  created_at: number;
  updated_at: number;
}

export interface ChunkView {
  idx: number;
  begin: number;
  end: number;
  state: "pending" | "leased" | "uploaded" | "verified";
  bytes_confirmed: number;
  adapter_token: string | null;
}

export interface JobSnapshot {
  job: JobRecord;
  chunks: ChunkView[];
  events: [number, string, string][];
}

/** The shareable link (the destination lane): a presigned GET minted by
 *  the shell from the journaled endpoint + the vault's keys. `expires_at`
 *  is epoch seconds — the UI shows the honest deadline. */
export interface ShareLink {
  url: string;
  expires_at: number;
}

/** The telemetry Privacy view (#75): consent + the local buffer's shape. */
export interface TelemetryStatus {
  consent: boolean;
  count: number;
  path: string;
}

/** What a hot apply did (#81): the registry after the upsert/remove. */
export interface ApplyOutcome {
  action: "replaced" | "added" | "removed";
  lanes: string[];
}

/** What Test-connection learned (#81). `final` sentences render as-is;
 * the rest route through the copy table with the raw preserved. */
export interface ProbeResult {
  checked: boolean;
  ok: boolean;
  sentence: string;
  final: boolean;
}

export const api = {
  engineStatus: (): Promise<string> => invoke("engine_status"),
  /** Registry names for the adapter select — the UI reflects the engine. */
  listAdapters: (): Promise<string[]> => invoke("list_adapters"),
  createJob: (spec: JobSpec, maxConns: number): Promise<JobRecord> =>
    invoke("create_job", { spec, maxConns }),
  listJobs: (): Promise<JobRecord[]> => invoke("list_jobs"),
  snapshot: (jobId: string): Promise<JobSnapshot> =>
    invoke("snapshot", { jobId }),
  /** Spec Sprint-1 item 9 name for the per-chunk inspector drawer (#16). */
  inspectChunks: (jobId: string): Promise<JobSnapshot> =>
    invoke("inspect_chunks", { jobId }),
  /** The shareable link (the destination lane): presigned GET for a
   *  FINISHED s3 job — 24 h TTL, minted on demand, never stored. The
   *  shell refuses anything else (other lanes have no link semantics). */
  shareLink: (jobId: string): Promise<ShareLink> =>
    invoke("share_link", { jobId }),
  pauseJob: (jobId: string): Promise<void> => invoke("pause_job", { jobId }),
  resumeSignal: (jobId: string): Promise<void> =>
    invoke("resume_signal", { jobId }),
  /** Honest cancel: journals `Canceled`, never `Failed` (#15). */
  cancelJob: (jobId: string): Promise<JobRecord> =>
    invoke("cancel_job", { jobId }),
  /** The shell stats, the engine plans — the UI never computes truth. */
  statFile: (path: string): Promise<number> => invoke("stat_file", { path }),
  // --- settings: the credentials vault (#69) -------------------------------
  /** The static schema — which adapters need which fields. */
  credentialsSchema: (): Promise<AdapterSchema[]> => invoke("credentials_schema"),
  /** Masked views for every schema field; secret values are never sent. */
  credentialsGet: (): Promise<AdapterViews> => invoke("credentials_get"),
  /** PARTIAL merge: absent field = keep, empty = delete (schema-guarded). */
  credentialsSet: (adapter: string, fields: Record<string, string>): Promise<void> =>
    invoke("credentials_set", { adapter, fields }),
  credentialsDelete: (adapter: string): Promise<void> =>
    invoke("credentials_delete", { adapter }),
  // --- painless auth (#81) -------------------------------------------------
  /** Save/remove side effect: rebuild the lane from vault+env and upsert
   *  it — the next job uses the fresh config, no restart. */
  applyCredentials: (adapter: string): Promise<ApplyOutcome> =>
    invoke("apply_credentials", { adapter }),
  /** The browser sign-in (#82): the shell spawns `rclone authorize
   *  <backend>` — the provider's real page opens in the user's browser,
   *  they sign in as normal, the captured token is merged into the bridge
   *  lane's extra_env INSIDE the shell (masked values never cross the UI)
   *  and the lane hot-applies. The backend id is allowlisted both ends. */
  authorizeOauth: (backend: string, bin?: string): Promise<ApplyOutcome> =>
    invoke("authorize_oauth", { backend, bin: bin ?? null }),
  /** The YouTube sign-in (#85): the shell runs its own loopback OAuth
   *  flow (PKCE + state) — the browser opens Google's real consent screen,
   *  the redirect returns to localhost:8737, and the exchanged token is
   *  stored under the lane's own hidden secret (OS keychain). The client
   *  id/secret are drafts (falling back to the saved values) — the honest
   *  one-time setup until IUM ships a registered client. */
  connectYoutube: (clientId?: string, clientSecret?: string): Promise<ApplyOutcome> =>
    invoke("connect_youtube", { clientId: clientId ?? null, clientSecret: clientSecret ?? null }),
  /** The Dropbox sign-in (#109): the shell runs its own loopback OAuth
   *  flow on IUM's OWN OAuth identity — a PKCE PUBLIC client (no secret
   *  exists on the lane). No fields to fill: the browser opens Dropbox's
   *  real consent page, the redirect returns to localhost:8738 (the fixed
   *  registered port), and the grant (offline refresh token included) is
   *  stored under the lane's hidden secret (OS keychain). */
  connectDropbox: (): Promise<ApplyOutcome> => invoke("connect_dropbox", {}),
  /** Read-only credential check against the DRAFTS (never persisted, no
   *  upload session created — a Test button cannot orphan a real upload). */
  probeCredentials: (adapter: string, fields: Record<string, string>): Promise<ProbeResult> =>
    invoke("probe_credentials", { adapter, fields }),
  /** Where the secret values actually live on this machine (footer marker). */
  secretMode: (): Promise<"keyring" | "file"> => invoke("secret_mode"),
  /** The materials verdict (#92): true only on Windows 11 (the engine's
   *  OS-build gate + a successful Mica apply). Everything else stays opaque. */
  platformMaterial: (): Promise<boolean> => invoke("platform_material"),
  // --- privacy: the telemetry stance (#66 lane 7, #75) ----------------------
  /** The OS notification mouth (#93): the shell decides when + the words
   *  (the pure rule in notify.ts); false = the OS refused (no daemon, a
   *  denied permission) — the shell never depends on this surface. */
  notifyUser: (title: string, body: string): Promise<boolean> =>
    invoke("notify_user", { title, body }),
  /** The speed cap (#94): bytes/sec across ALL jobs and lanes, or null for
   *  unlimited. HOT — running jobs obey on their next between-lease
   *  acquire. The shell persists the choice and re-asserts it at boot. */
  setRateCap: (cap: number | null): Promise<void> =>
    invoke("set_rate_cap", { cap }),
  /** Consent switch state + the local buffer's count and absolute path. */
  telemetryStatus: (): Promise<TelemetryStatus> => invoke("telemetry_status"),
  /** The master switch — enforced at the engine's single write point. */
  telemetrySetConsent: (on: boolean): Promise<void> =>
    invoke("telemetry_set_consent", { on }),
  /** Empties the local buffer (the UI two-step-confirms). */
  telemetryClear: (): Promise<void> => invoke("telemetry_clear"),
};
