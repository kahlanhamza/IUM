import { useEffect, useState } from "react";
import { api, type JobRecord } from "./api";

/**
 * Sprint-0 shell: proves the IPC contract end-to-end.
 * The full jobs table with badges + speed graph lands in Sprint 1 (spec §8).
 */
export default function App() {
  const [status, setStatus] = useState("connecting…");
  const [jobs, setJobs] = useState<JobRecord[]>([]);

  useEffect(() => {
    api
      .engineStatus()
      .then(setStatus)
      .catch((e) => setStatus(`engine unreachable: ${e}`));
    api
      .listJobs()
      .then(setJobs)
      .catch(() => setJobs([]));
  }, []);

  return (
    <main style={{ fontFamily: "system-ui, sans-serif", padding: 24 }}>
      <h1>IUM — Internet Upload Manager</h1>
      <p>
        engine: <strong>{status}</strong>
      </p>
      <h2>Jobs</h2>
      {jobs.length === 0 ? (
        <p style={{ color: "#666" }}>
          No jobs yet. Drag-and-drop destinations arrive in Sprint 1.
        </p>
      ) : (
        <table cellPadding={6}>
          <thead>
            <tr>
              <th>File</th>
              <th>Adapter</th>
              <th>Badge</th>
              <th>State</th>
            </tr>
          </thead>
          <tbody>
            {jobs.map((j) => (
              <tr key={j.id}>
                <td>{j.source_path}</td>
                <td>{j.adapter}</td>
                <td>{j.capability_badge === "resumable" ? "Resumable ✅" : "One-shot ⚠️"}</td>
                <td>{j.state}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </main>
  );
}
