/**
 * Typed IPC contract — the upload twin of Motrix's MBP1 bridge lesson:
 * one stable, versioned surface between UI and engine (spec §2.1).
 * Mirrors the #[tauri::command]s in src-tauri/src/lib.rs.
 */
import { invoke } from "@tauri-apps/api/core";

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
  createJob: (spec: JobSpec, maxConns: number): Promise<JobRecord> =>
    invoke("create_job", { spec, maxConns }),
  listJobs: (): Promise<JobRecord[]> => invoke("list_jobs"),
  snapshot: (jobId: string): Promise<JobSnapshot> =>
    invoke("snapshot", { jobId }),
  pauseJob: (jobId: string): Promise<void> => invoke("pause_job", { jobId }),
  resumeSignal: (jobId: string): Promise<void> =>
    invoke("resume_signal", { jobId }),
};
