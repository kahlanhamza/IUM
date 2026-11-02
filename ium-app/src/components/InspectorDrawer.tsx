/**
 * Per-chunk inspector drawer (spec §8: aria2-style transparency, issue #16).
 * Renders `inspect_chunks`/`job_updated` snapshots verbatim — the engine's
 * journal is the only truth on display.
 */
import type { JobSnapshot } from "../api";

export interface InspectorDrawerProps {
  snapshot: JobSnapshot | null;
  onClose: () => void;
}

export function InspectorDrawer({ snapshot, onClose }: InspectorDrawerProps) {
  if (!snapshot) return null;
  const { job, chunks, events } = snapshot;
  return (
    <aside style={drawer}>
      <div style={head}>
        <strong>Job inspector</strong>
        <button onClick={onClose}>Close</button>
      </div>
      <div style={mono}>
        id: {job.id}
        <br />
        file: {job.source_path}
        <br />
        endpoint: {job.endpoint}
      </div>
      <h3 style={h3}>Chunks</h3>
      <table cellPadding={4} style={{ borderCollapse: "collapse", width: "100%" }}>
        <thead>
          <tr>
            <th style={th}>#</th>
            <th style={th}>Span</th>
            <th style={th}>State</th>
            <th style={th}>Confirmed</th>
            <th style={th}>Token</th>
          </tr>
        </thead>
        <tbody>
          {chunks.map((c) => (
            <tr key={c.idx}>
              <td style={td}>{c.idx}</td>
              <td style={td}>
                {c.begin}–{c.end}
              </td>
              <td style={td}>{c.state}</td>
              <td style={td}>{c.bytes_confirmed}</td>
              <td style={{ ...td, fontFamily: "ui-monospace, monospace", fontSize: 11 }}>
                {c.adapter_token ?? "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3 style={h3}>Journal events</h3>
      <ol style={{ margin: 0, paddingLeft: 20, maxHeight: 200, overflowY: "auto" }}>
        {events.map(([seq, kind, payload]) => (
          <li key={seq} style={{ fontFamily: "ui-monospace, monospace", fontSize: 11 }}>
            #{seq} {kind} <span style={{ color: "#666" }}>{payload}</span>
          </li>
        ))}
      </ol>
    </aside>
  );
}

const drawer: React.CSSProperties = {
  position: "fixed",
  top: 0,
  right: 0,
  bottom: 0,
  width: 420,
  background: "#fff",
  borderLeft: "2px solid #d0d4cf",
  boxShadow: "-4px 0 16px rgba(0,0,0,0.08)",
  padding: 16,
  overflowY: "auto",
};

const head: React.CSSProperties = { display: "flex", justifyContent: "space-between", alignItems: "center" };
const h3: React.CSSProperties = { margin: "14px 0 6px" };
const th: React.CSSProperties = { textAlign: "left", borderBottom: "1px solid #d0d4cf" };
const td: React.CSSProperties = { borderBottom: "1px solid #eceff1", fontSize: 12 };
const mono: React.CSSProperties = { fontFamily: "ui-monospace, monospace", fontSize: 11, color: "#444" };
