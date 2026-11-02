/**
 * Jobs table (spec §8 first deliverable, issue #16): file, destination +
 * honest badge, state chip, progress bar fed by `job_updated` snapshots,
 * live speed as Δconfirmed/Δt, and action buttons gated by the engine's
 * legal transitions (mirrored in logic.ts, tested in logic.test.ts).
 */
import type { JobRecord } from "../api";
import { badgeFor, canCancel, canPause, canResume, hostOf, humanBytes, percentDone } from "../logic";

export interface JobView {
  record: JobRecord;
  /** Σ bytes_confirmed from the latest snapshot (journal truth). */
  confirmed: number;
  /** Δconfirmed/Δt over the last two snapshots (presentation math). */
  speed: number;
}

export interface JobsTableProps {
  jobs: JobView[];
  selected: string | null;
  onSelect: (jobId: string) => void;
  onPause: (jobId: string) => void;
  onResume: (jobId: string) => void;
  onCancel: (jobId: string) => void;
}

export function JobsTable({ jobs, selected, onSelect, onPause, onResume, onCancel }: JobsTableProps) {
  if (jobs.length === 0) {
    return <p style={{ color: "#666" }}>No jobs yet — create one above.</p>;
  }
  return (
    <table cellPadding={6} style={{ borderCollapse: "collapse", width: "100%" }}>
      <thead>
        <tr style={headRow}>
          <th style={th}>File</th>
          <th style={th}>Destination</th>
          <th style={th}>Badge</th>
          <th style={th}>State</th>
          <th style={th}>Progress</th>
          <th style={th}>Speed</th>
          <th style={th}>Actions</th>
        </tr>
      </thead>
      <tbody>
        {jobs.map((j) => {
          const { record } = j;
          const pct = percentDone(j.confirmed, record.source_size, record.state);
          return (
            <tr
              key={record.id}
              onClick={() => onSelect(record.id)}
              style={{ cursor: "pointer", background: selected === record.id ? "#eef4f2" : undefined }}
            >
              <td style={td}>
                {fileName(record.source_path)}
                <div style={dim}>{humanBytes(record.source_size)}</div>
              </td>
              <td style={td}>
                {record.adapter}
                <div style={dim}>{hostOf(record.endpoint)}</div>
              </td>
              <td style={td}>{badgeFor(record.capability_badge)}</td>
              <td style={td}>
                <span style={stateChip(record.state)}>{record.state}</span>
              </td>
              <td style={{ ...td, minWidth: 180 }}>
                <div style={barTrack}>
                  <div style={{ ...barFill, width: `${pct}%` }} />
                </div>
                <div style={dim}>
                  {pct}% · {humanBytes(j.confirmed)} / {humanBytes(record.source_size)}
                </div>
              </td>
              <td style={td}>{j.speed > 0 ? `${humanBytes(j.speed)}/s` : "—"}</td>
              <td style={td}>
                <button
                  disabled={!canPause(record.state)}
                  onClick={(e) => {
                    e.stopPropagation();
                    onPause(record.id);
                  }}
                >
                  Pause
                </button>{" "}
                <button
                  disabled={!canResume(record.state)}
                  onClick={(e) => {
                    e.stopPropagation();
                    onResume(record.id);
                  }}
                >
                  Resume
                </button>{" "}
                <button
                  disabled={!canCancel(record.state)}
                  onClick={(e) => {
                    e.stopPropagation();
                    onCancel(record.id);
                  }}
                >
                  Cancel
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function fileName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i >= 0 ? path.slice(i + 1) : path;
}

function stateChip(state: string): React.CSSProperties {
  const bg =
    state === "done"
      ? "#e3f2e3"
      : state === "failed" || state === "unresumable"
        ? "#fbe4e2"
        : state === "canceled"
          ? "#efe7f6"
          : state === "paused"
            ? "#fff3d6"
            : "#e8f0fe";
  return {
    background: bg,
    borderRadius: 10,
    padding: "2px 8px",
    fontSize: 12,
    fontFamily: "ui-monospace, monospace",
  };
}

const headRow: React.CSSProperties = { textAlign: "left", borderBottom: "2px solid #d0d4cf" };
const th: React.CSSProperties = { textAlign: "left", padding: "6px 8px" };
const td: React.CSSProperties = { borderTop: "1px solid #eceff1", padding: "6px 8px", verticalAlign: "top" };
const dim: React.CSSProperties = { color: "#666", fontSize: 12 };
const barTrack: React.CSSProperties = { height: 8, background: "#eceff1", borderRadius: 4, overflow: "hidden" };
const barFill: React.CSSProperties = { height: "100%", background: "#2ec4b6" };
