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
};
