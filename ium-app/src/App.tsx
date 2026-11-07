/**
 * IUM shell (#16 spec slice; #65 Lane 1 — the shipped-product pass).
 *
 * Everything shown is a journal snapshot (`job_updated` events carry the
 * same `JobSnapshot` a poll would return); the UI's only job is to not lie
 * about what the engine says. The redesign changes presentation only — the
 * event math, the honest badges, and the e2e seams are untouched.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type JobRecord, type JobSnapshot } from "./api";
import { CreateJobForm } from "./components/CreateJobForm";
import { InspectorDrawer } from "./components/InspectorDrawer";
import { JobsTable, type JobView } from "./components/JobsTable";
import { IconAlert, IconCheck, IconGear, IconUpload, Logomark } from "./components/icons";
import { SettingsDialog } from "./components/SettingsDialog";
import { confirmedTotal, recentEndpoints, speedBetween } from "./logic";
import { APP_COPY, humanizeError } from "./copy";
import { checkForUpdate } from "./updater";

interface Progress {
  confirmed: number;
  speed: number;
}

type EngineState =
  | { kind: "connecting" }
  | { kind: "ready"; detail: string }
  | { kind: "down"; detail: string };

interface Toast {
  id: number;
  kind: "error" | "ok";
  text: string;
}

export default function App() {
  const [engine, setEngine] = useState<EngineState>({ kind: "connecting" });
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const [inspected, setInspected] = useState<JobSnapshot | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Last snapshot per job for the Δ speed math — kept in a ref so the event
  // handler never goes stale.
  const lastSample = useRef<Record<string, { confirmed: number; ms: number }>>({});
  const toastSeq = useRef(0);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    const id = ++toastSeq.current;
    setToasts((cur) => [...cur.slice(-3), { id, kind, text }]);
    window.setTimeout(() => {
      setToasts((cur) => cur.filter((t) => t.id !== id));
    }, 5200);
  }, []);

  const onError = useCallback(
    // The copy table (#70): toasts carry the human sentence; the form alert
    // keeps the raw engine words in view.
    (message: string) => pushToast("error", humanizeError(message).title),
    [pushToast],
  );

  const refreshJobs = useCallback(() => {
    api.listJobs().then((records) => {
      setJobs(records);
      // Live jobs speak through job_updated; every other state would sit at
      // 0% forever — no event ever comes. Read their truth from the journal
      // instead (#77: the product screenshots exposed the 0%-done lie).
      for (const r of records) {
        if (r.state === "running" || r.state === "queued") continue;
        api
          .snapshot(r.id)
          .then((s) =>
            setProgress((prev) => ({
              ...prev,
              [r.id]: { confirmed: confirmedTotal(s.chunks), speed: 0 },
            })),
          )
          .catch(() => {
            /* a vanished row stays at zero — the next refresh retries */
          });
      }
    }).catch(() => setJobs([]));
  }, []);

  useEffect(() => {
    api
      .engineStatus()
      .then((detail) => setEngine({ kind: "ready", detail }))
      .catch((e) => {
        setEngine({ kind: "down", detail: String(e) });
        pushToast("error", humanizeError(`engine unreachable: ${e}`).title);
      });
    refreshJobs();

    // The silent startup update check (#80): once per boot, a few seconds
    // in, Tauri-only (the wrapper degrades to null elsewhere). A failed
    // check stays silent HERE — the About control is where an error becomes
    // a sentence; the chrome never nags on a flaky check.
    const updateTimer = setTimeout(() => {
      void checkForUpdate()
        .then((info) => {
          if (info) {
            pushToast("ok", APP_COPY.updateAvailable.replace("{version}", info.version));
          }
        })
        .catch(() => {});
    }, 4000);

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
      clearTimeout(updateTimer);
      unlisten.then((f) => f()).catch(() => {
        /* no Tauri runtime (plain-browser dev) — nothing to unlisten */
      });
    };
  }, [refreshJobs, pushToast]);

  // The inspector reads the journal directly when opened (one honest fetch).
  useEffect(() => {
    if (!selected) {
      setInspected(null);
      return;
    }
    api
      .inspectChunks(selected)
      .then(setInspected)
      .catch(() => setInspected(null));
  }, [selected]);

  const views: JobView[] = jobs.map((record) => ({
    record,
    confirmed: progress[record.id]?.confirmed ?? 0,
    speed: progress[record.id]?.speed ?? 0,
  }));

  const activeCount = jobs.filter((j) => !isTerminalState(j.state)).length;

  return (
    <div className="app">
      <header className="app-header">
        <div className="brand">
          {/* The constructed mark (#77) — identity in the chrome, not a
              repurposed action icon. */}
          <Logomark className="logomark" size={28} />
          <div>
            <div className="brand-name">IUM</div>
            <div className="brand-sub">Internet Upload Manager</div>
          </div>
        </div>
        {/* Quiet status (#77): dot + two words — the big-tech rule (HIG,
            defer to content). The engine's own sentence stays inspectable in
            the tooltip and Settings → About, never as permanent chrome. */}
        <div
          className={`engine-pill engine-pill--${
            engine.kind === "ready" ? "ok" : engine.kind === "connecting" ? "wait" : "down"
          }`}
          title={
            engine.kind === "connecting"
              ? "connecting to the engine…"
              : engine.detail
          }
        >
          <span className="dot" />
          {engine.kind === "ready"
            ? "Connected"
            : engine.kind === "connecting"
              ? "Connecting…"
              : "Engine unreachable"}
        </div>
        {/* The credentials vault (#69): a product surface, not env vars.
            id is a pinned e2e seam (the locator inventory). */}
        <button
          id="app-settings"
          className="btn btn--ghost btn--icon"
          aria-label="Open settings — adapter credentials"
          title="Settings"
          onClick={() => setSettingsOpen(true)}
        >
          <IconGear size={16} />
        </button>
      </header>

      <main className="app-main">
        <CreateJobForm
          onCreated={() => {
            refreshJobs();
            pushToast("ok", APP_COPY.created);
          }}
          recents={recentEndpoints(jobs)}
          onError={onError}
        />

        <div className="section-head">
          <h2 className="section-title">Uploads</h2>
          {jobs.length > 0 && (
            <span className="count-chip">
              {jobs.length} total{activeCount > 0 ? ` · ${activeCount} active` : ""}
            </span>
          )}
        </div>

        {views.length === 0 ? (
          <div className="card">
            <div className="empty">
              <IconUpload size={26} />
              <div className="empty-title">{APP_COPY.emptyTitle}</div>
              <div className="empty-hint">{APP_COPY.emptyHint}</div>
            </div>
          </div>
        ) : (
          <div className="card" style={{ padding: 0, overflow: "hidden" }}>
            <JobsTable
              jobs={views}
              selected={selected}
              onSelect={setSelected}
              onPause={(id) =>
                api.pauseJob(id).then(refreshJobs).catch((e) => onError(`pause failed: ${e}`))
              }
              onResume={(id) =>
                api
                  .resumeSignal(id)
                  .then(refreshJobs)
                  .catch((e) => onError(`resume failed: ${e}`))
              }
              onCancel={(id) =>
                api.cancelJob(id).then(refreshJobs).catch((e) => onError(`cancel failed: ${e}`))
              }
            />
          </div>
        )}
      </main>

      <InspectorDrawer snapshot={inspected} onClose={() => setSelected(null)} />

      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onError={onError}
        onSaved={(message) => pushToast("ok", message)}
        engineDetail={engine.kind === "connecting" ? undefined : engine.detail}
      />

      <div className="toast-stack" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast--${t.kind}`}>
            {t.kind === "error" ? (
              <IconAlert className="toast-icon" size={15} />
            ) : (
              <IconCheck className="toast-icon" size={15} />
            )}
            <span>{t.text}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function isTerminalState(state: string): boolean {
  return ["done", "failed", "canceled", "unresumable"].includes(state);
}
