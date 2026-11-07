/**
 * Jobs table (#16 first deliverable; #65 Lane 1 visual pass).
 *
 * The semantic structure is LOAD-BEARING for the app-e2e lane and pinned on
 * issue #65: rows are located via `table tbody tr:nth-child(i)`, the file
 * name lives in the FIRST td, the FIFTH td's text starts with `NN%`, and
 * every row carries a `[data-state]` chip. Restyle anything except that.
 *
 * Progress bar + speed are presentation math over journal snapshots; the
 * buttons mirror the engine's legal transitions (tested in logic.test.ts).
 */
import type { JobRecord } from "../api";
import {
  badgeFor,
  canCancel,
  canPause,
  canResume,
  friendlyAdapter,
  hostOf,
  humanBytes,
  isTerminal,
  percentDone,
} from "../logic";
import { IconAlert, IconInfinite, IconPause, IconPlay, IconFile, IconX } from "./icons";
import { Badge, Button, Chip } from "./ui";

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
  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            <th>File</th>
            <th>Destination</th>
            <th>Mode</th>
            <th>State</th>
            <th>Progress</th>
            <th>Speed</th>
            <th>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {jobs.map((j) => {
            const { record } = j;
            const pct = percentDone(j.confirmed, record.source_size, record.state);
            const badge = badgeFor(record.capability_badge);
            return (
              <tr
                key={record.id}
                data-active={!isTerminal(record.state)}
                onClick={() => onSelect(record.id)}
                className={selected === record.id ? "row--selected" : undefined}
              >
                <td>
                  <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                    <IconFile size={15} style={{ flex: "none", color: "var(--text-3)" }} />
                    <div style={{ minWidth: 0 }}>
                      <div className="cell-main">{fileName(record.source_path)}</div>
                      <div className="cell-dim">{humanBytes(record.source_size)}</div>
                    </div>
                  </div>
                </td>
                <td>
                  <div className="cell-main">{friendlyAdapter(record.adapter)}</div>
                  <div className="cell-dim">{hostOf(record.endpoint)}</div>
                </td>
                <td>
                  <Badge
                    kind={badge.kind}
                    title={
                      badge.kind === "resumable"
                        ? "This destination accepts resumable uploads — interruptions heal."
                        : badge.kind === "one_shot"
                          ? "One-shot destination: an interruption restarts from zero."
                          : undefined
                    }
                  >
                    {badge.kind === "resumable" ? (
                      <IconInfinite size={11} />
                    ) : badge.kind === "one_shot" ? (
                      <IconAlert size={11} />
                    ) : null}
                    {badge.label}
                  </Badge>
                </td>
                <td>
                  {/* data-state: the journal's truth as a WebDriver-readable
                      attribute (#35) — the chip's text stays human-facing. */}
                  <Chip state={record.state}>{record.state}</Chip>
                </td>
                <td className="cell-progress">
                  {/* #67 RED fix: a progressbar without a name announces
                      "progress bar, 25" — name it after WHAT is progressing. */}
                  <div
                    className="progress"
                    role="progressbar"
                    aria-label={`${fileName(record.source_path)}: ${pct}% uploaded`}
                    aria-valuenow={pct}
                    aria-valuemin={0}
                    aria-valuemax={100}
                  >
                    <div className="progress-fill" style={{ width: `${pct}%` }} />
                  </div>
                  <div className="cell-dim">
                    {pct}% · {humanBytes(j.confirmed)} / {humanBytes(record.source_size)}
                  </div>
                </td>
                <td className="cell-speed">{j.speed > 0 ? `${humanBytes(j.speed)}/s` : "—"}</td>
                <td>
                  <div className="cell-actions">
                    {canResume(record.state) ? (
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`resume job ${record.id}`}
                        title="Resume"
                        disabled={!canResume(record.state)}
                        onClick={(e) => {
                          e.stopPropagation();
                          onResume(record.id);
                        }}
                      >
                        <IconPlay size={13} />
                      </Button>
                    ) : (
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`pause job ${record.id}`}
                        title="Pause"
                        disabled={!canPause(record.state)}
                        onClick={(e) => {
                          e.stopPropagation();
                          onPause(record.id);
                        }}
                      >
                        <IconPause size={13} />
                      </Button>
                    )}
                    <Button
                      variant="danger-ghost"
                      size="icon"
                      aria-label={`cancel job ${record.id}`}
                      title="Cancel"
                      disabled={!canCancel(record.state)}
                      onClick={(e) => {
                        e.stopPropagation();
                        onCancel(record.id);
                      }}
                    >
                      <IconX size={13} />
                    </Button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function fileName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i >= 0 ? path.slice(i + 1) : path;
}
