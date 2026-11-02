/**
 * Sprint-1 UI slice (spec §8, issue #16): create form, jobs table with live
 * progress + honest badges + engine-mirrored actions, per-chunk inspector.
 *
 * Everything shown is a journal snapshot (`job_updated` events carry the
 * same `JobSnapshot` a poll would return); the UI's only job is to not lie
 * about what the engine says.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type JobRecord, type JobSnapshot } from "./api";
import { CreateJobForm } from "./components/CreateJobForm";
import { InspectorDrawer } from "./components/InspectorDrawer";
import { JobsTable, type JobView } from "./components/JobsTable";
import { confirmedTotal, speedBetween } from "./logic";

interface Progress {
  confirmed: number;
  speed: number;
}

export default function App() {
  const [status, setStatus] = useState("connecting…");
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const [inspected, setInspected] = useState<JobSnapshot | null>(null);

  // Last snapshot per job for the Δ speed math — kept in a ref so the event
  // handler never goes stale.
  const lastSample = useRef<Record<string, { confirmed: number; ms: number }>>({});

  const refreshJobs = useCallback(() => {
    api
      .listJobs()
      .then(setJobs)
      .catch(() => setJobs([]));
  }, []);

  useEffect(() => {
    api
      .engineStatus()
      .then(setStatus)
      .catch((e) => setStatus(`engine unreachable: ${e}`));
    refreshJobs();

    const unlisten = listen<JobSnapshot>("job_updated", (e) => {
      const snap = e.payload;
      setJobs((prev) => {
        const i = prev.findIndex((j) => j.id === snap.job.id);
        if (i < 0) return [...prev, snap.job];
        const next = [...prev];
        next[i] = snap.job;
        return next;
      });
      const now = Date.now();
      const confirmed = confirmedTotal(snap.chunks);
      const before = lastSample.current[snap.job.id];
      const speed = before
        ? speedBetween(confirmed, before.confirmed, now, before.ms)
        : 0;
      lastSample.current[snap.job.id] = { confirmed, ms: now };
      setProgress((prev) => ({ ...prev, [snap.job.id]: { confirmed, speed } }));
      setInspected((cur) => (cur && cur.job.id === snap.job.id ? snap : cur));
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, [refreshJobs]);

  // The inspector reads the journal directly when opened (one honest fetch).
  useEffect(() => {
    if (!selected) {
      setInspected(null);
      return;
    }
    api.inspectChunks(selected).then(setInspected).catch(() => setInspected(null));
  }, [selected]);

  const views: JobView[] = jobs.map((record) => ({
    record,
    confirmed: progress[record.id]?.confirmed ?? 0,
    speed: progress[record.id]?.speed ?? 0,
  }));

  return (
    <main style={{ fontFamily: "system-ui, sans-serif", padding: 24, maxWidth: 1100 }}>
      <h1>IUM — Internet Upload Manager</h1>
      <p>
        engine: <strong>{status}</strong>
      </p>

      <CreateJobForm
        onCreated={() => {
          refreshJobs();
        }}
      />

      <h2>Jobs</h2>
      <JobsTable
        jobs={views}
        selected={selected}
        onSelect={setSelected}
        onPause={(id) => api.pauseJob(id).then(refreshJobs).catch(() => refreshJobs())}
        onResume={(id) => api.resumeSignal(id).then(refreshJobs).catch(() => refreshJobs())}
        onCancel={(id) => api.cancelJob(id).then(refreshJobs).catch(() => refreshJobs())}
      />

      <InspectorDrawer snapshot={inspected} onClose={() => setSelected(null)} />
    </main>
  );
}
