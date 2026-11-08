/**
 * The docked inspector (#88; spec §8: aria2-style transparency, #16).
 * Docks at the window's bottom edge and PUSHES the content — never an
 * overlay, never position:fixed (pinned by anatomy.test.tsx). Renders
 * `inspect_chunks`/`job_updated` snapshots verbatim — the engine's journal
 * is the only truth on display.
 *
 * The chunks render as the mockup's grid: one square per chunk, carrying
 * the journal's own words as data attributes — `data-idx`, `data-state`
 * (the raw state), `data-confirmed` (bytes). That grid IS the e2e seam
 * (the #65 table leg was amended to it in the same commit as this pane).
 */
import { useEffect } from "react";
import type { JobSnapshot } from "../api";
import { friendlyAdapter, humanBytes, hostOf } from "../logic";
import { IconX } from "./icons";
import { Button } from "./ui";

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
  const confirmedCount = chunks.filter(
    (c) => c.state === "verified" || c.state === "uploaded",
  ).length;

  return (
    // #67 RED fix: `role="dialog"` is not allowed on <aside> — strict AT
    // implementations drop the role entirely. A <div> carries it. The
    // docked pane is a NON-modal dialog: the library stays reachable.
    <div className="inspector" role="dialog" aria-label={`Inspector for job ${job.id}`}>
      <div className="ihead">
        <div style={{ minWidth: 0 }}>
          <span className="iname">{fileName(job.source_path)}</span>{" "}
          <span className="ipath">
            {job.source_path} · {hostOf(job.endpoint)}
          </span>
        </div>
        <Button variant="ghost" size="icon" aria-label="Close inspector" onClick={onClose}>
          <IconX size={14} />
        </Button>
      </div>

      <div className="icols">
        <div className="icol">
          <div className="cap">Job</div>
          <div className="mline">{job.id}</div>
          <div className="mline">
            {humanBytes(job.source_size)} · {friendlyAdapter(job.adapter)} · {job.capability_badge}
          </div>
          <div className="mline jseq">{job.endpoint}</div>
        </div>

        <div className="icol">
          <div className="cap">
            Chunks — {confirmedCount}/{chunks.length} confirmed ·{" "}
            {humanBytes(chunks.reduce((acc, c) => acc + c.bytes_confirmed, 0))} of{" "}
            {humanBytes(job.source_size)}
          </div>
          <div className="chunks">
            {chunks.map((c) => (
              <div
                key={c.idx}
                className={`c${c.state === "verified" || c.state === "uploaded" ? " done" : c.state === "leased" ? " act" : ""}`}
                data-idx={c.idx}
                data-state={c.state}
                data-confirmed={c.bytes_confirmed}
                title={`chunk ${c.idx} — ${c.state} — ${humanBytes(c.bytes_confirmed)} confirmed`}
              />
            ))}
          </div>
        </div>

        <div className="icol">
          <div className="cap">Journal</div>
          <ol className="journal">
            {events.map(([seq, kind, payload]) => (
              <li key={seq}>
                <span className="seq">#{seq}</span> <span className="kind">{kind}</span>{" "}
                <span className="payload">{payload}</span>
              </li>
            ))}
          </ol>
        </div>
      </div>
    </div>
  );
}

function fileName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i >= 0 ? path.slice(i + 1) : path;
}
