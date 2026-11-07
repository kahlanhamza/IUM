/**
 * Per-chunk inspector drawer (spec §8: aria2-style transparency, #16;
 * #65 Lane 1 visual pass). Renders `inspect_chunks`/`job_updated` snapshots
 * verbatim — the engine's journal is the only truth on display.
 */
import { useEffect } from "react";
import type { JobSnapshot } from "../api";
import { friendlyAdapter, humanBytes, hostOf } from "../logic";
import { IconX } from "./icons";
import { Button, Chip } from "./ui";

export interface InspectorDrawerProps {
  snapshot: JobSnapshot | null;
  onClose: () => void;
}

export function InspectorDrawer({ snapshot, onClose }: InspectorDrawerProps) {
  // Escape closes — a desktop-app reflex, not a web-page one.
  useEffect(() => {
    if (!snapshot) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [snapshot, onClose]);

  if (!snapshot) return null;
  const { job, chunks, events } = snapshot;
  return (
    <>
      <div className="drawer-backdrop" onClick={onClose} aria-hidden="true" />
      {/* #67 RED fix: `role="dialog"` is not allowed on <aside> — strict AT
          implementations drop the role entirely. A <div> carries it. */}
      <div className="drawer" role="dialog" aria-label={`Inspector for job ${job.id}`}>
        <div className="drawer-head">
          <div style={{ minWidth: 0 }}>
            <h2 className="drawer-title">{fileName(job.source_path)}</h2>
            <div className="drawer-sub">{hostOf(job.endpoint)}</div>
          </div>
          <Button variant="ghost" size="icon" aria-label="Close inspector" onClick={onClose}>
            <IconX size={14} />
          </Button>
        </div>

        <div className="meta-grid">
          <span className="meta-key">Job</span>
          <span className="meta-val">{job.id}</span>
          <span className="meta-key">Size</span>
          <span className="meta-val">{humanBytes(job.source_size)}</span>
          <span className="meta-key">Lane</span>
          <span className="meta-val">
            {friendlyAdapter(job.adapter)} · {job.capability_badge}
          </span>
          <span className="meta-key">Endpoint</span>
          <span className="meta-val">{job.endpoint}</span>
        </div>

        <h3 className="drawer-h3">Chunks</h3>
        <div className="table-wrap">
          <table className="chunk-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Span</th>
                <th>State</th>
                <th>Confirmed</th>
                <th>Token</th>
              </tr>
            </thead>
            <tbody>
              {chunks.map((c) => (
                <tr key={c.idx}>
                  <td>{c.idx}</td>
                  <td>
                    {humanBytes(c.begin)}–{humanBytes(c.end)}
                  </td>
                  <td>
                    <Chip state={chunkState(c.state)}>{c.state}</Chip>
                  </td>
                  <td>{c.bytes_confirmed}</td>
                  <td>{c.adapter_token ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <h3 className="drawer-h3">Journal events</h3>
        <ol className="journal">
          {events.map(([seq, kind, payload]) => (
            <li key={seq}>
              <span className="seq">#{seq}</span> <span className="kind">{kind}</span>{" "}
              <span className="payload">{payload}</span>
            </li>
          ))}
        </ol>
      </div>
    </>
  );
}

/** Chunk states map onto the chip palette; unknown stays neutral. */
function chunkState(state: string): string {
  return ["verified", "uploaded", "leased", "pending"].includes(state) ? state : "";
}

function fileName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i >= 0 ? path.slice(i + 1) : path;
}
