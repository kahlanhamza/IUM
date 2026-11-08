/**
 * IUM shell — the Ink anatomy (#88; #16 spec slice; #65 Lane 1).
 *
 * The four-pane contract (the Transmit/ForkLift lineage, the owner-approved
 * mockups): the 48 px toolbar COMMANDS, the 210 px sidebar NAVIGATES, the
 * content pane WORKS, the status bar + docked inspector EXPLAIN. The brand
 * block is gone from the working chrome (the logo ladder's Chrome rung:
 * none — identity lives at Launch/About, never over the work).
 *
 * Everything shown is a journal snapshot (`job_updated` events carry the
 * same `JobSnapshot` a poll would return); the UI's only job is to not lie
 * about what the engine says. The anatomy changes presentation only — the
 * event math, the honest badges, and the e2e seams are untouched.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type AdapterSchema, type JobRecord, type JobSnapshot } from "./api";
import { CommandPalette, modGlyph, type Command } from "./components/CommandPalette";
import { CreateJobForm } from "./components/CreateJobForm";
import { InspectorDrawer } from "./components/InspectorDrawer";
import { JobsTable, type JobView } from "./components/JobsTable";
import {
  IconAlert,
  IconCheck,
  IconCloud,
  IconGear,
  IconPlay,
  IconPlus,
  IconUpload,
  IconX,
} from "./components/icons";
import { SettingsDialog } from "./components/SettingsDialog";
import { ThemeToggle } from "./components/ThemeToggle";
import { confirmedTotal, parseVersion, recentEndpoints, speedBetween } from "./logic";
import { APP_COPY, humanizeError } from "./copy";
import { checkForUpdate } from "./updater";
import { useThemePref } from "./useTheme";

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

/** The sidebar Library's five rungs — the journal's states bucketed honestly. */
type Filter = "all" | "active" | "queued" | "completed" | "failed";

const FILTERS: Array<{ key: Filter; label: string }> = [
  { key: "all", label: "All uploads" },
  { key: "active", label: "Active" },
  { key: "queued", label: "Queued" },
  { key: "completed", label: "Completed" },
  { key: "failed", label: "Failed" },
];

/** `created` plans but has not started — that is what "queued" means. */
function inFilter(filter: Filter, state: string): boolean {
  switch (filter) {
    case "all":
      return true;
    case "active":
      return ["probing", "running", "finishing"].includes(state);
    case "queued":
      return state === "created";
    case "completed":
      return state === "done";
    case "failed":
      return state === "failed" || state === "unresumable";
  }
}

interface Lane {
  adapter: string;
  label: string;
  connected: boolean;
}

/** The vault's own truth, one field at a time: connected = every required
 *  field present (masked values never cross the IPC boundary either way). */
function laneConnected(schema: AdapterSchema, views: Awaited<ReturnType<typeof api.credentialsGet>>): boolean {
  const fields = views[schema.adapter];
  if (!fields) return false;
  return schema.fields
    .filter((f) => f.required)
    .every((f) => fields[f.name]?.present === true);
}

export default function App() {
  const [engine, setEngine] = useState<EngineState>({ kind: "connecting" });
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const [inspected, setInspected] = useState<JobSnapshot | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [filter, setFilter] = useState<Filter>("all");
  const [lanes, setLanes] = useState<Lane[]>([]);
  const [themePref, setThemePref] = useThemePref();

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

  // The Destinations dots are the vault's own truth (#88): schema × masked
  // presence. Re-read when Settings closes — connecting must light the dot.
  const refreshLanes = useCallback(() => {
    Promise.all([api.credentialsSchema(), api.credentialsGet()])
      .then(([schemas, views]) =>
        setLanes(
          schemas.map((s) => ({
            adapter: s.adapter,
            label: s.label,
            connected: laneConnected(s, views),
          })),
        ),
      )
      .catch(() => setLanes([]));
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
    refreshLanes();

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
  }, [refreshJobs, refreshLanes, pushToast]);

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

  const focusNewUpload = useCallback(() => {
    // The #upload-path id is a pinned e2e seam — the command bar's primary
    // action is a desktop-app reflex: put the cursor where typing starts.
    document.getElementById("upload-path")?.focus();
  }, []);

  // --- the ⌘K command palette (#91) — every command is REAL ----------------
  // The honest batch verbs: one engine call per job, results counted, and a
  // zero-candidate run says so (the palette never pretends).
  const pauseAllActive = useCallback(() => {
    const active = jobs.filter((r) => inFilter("active", r.state));
    if (active.length === 0) {
      pushToast("ok", APP_COPY.noActiveUploads);
      return;
    }
    Promise.allSettled(active.map((r) => api.pauseJob(r.id))).then((results) => {
      refreshJobs();
      const failed = results.filter((r) => r.status === "rejected").length;
      if (failed > 0) onError(`pause failed: ${failed} of ${active.length} job(s)`);
    });
  }, [jobs, refreshJobs, pushToast, onError]);

  const resumeAllPaused = useCallback(() => {
    const paused = jobs.filter((r) => r.state === "paused");
    if (paused.length === 0) {
      pushToast("ok", APP_COPY.noPausedUploads);
      return;
    }
    Promise.allSettled(paused.map((r) => api.resumeSignal(r.id))).then((results) => {
      refreshJobs();
      const failed = results.filter((r) => r.status === "rejected").length;
      if (failed > 0) onError(`resume failed: ${failed} of ${paused.length} job(s)`);
    });
  }, [jobs, refreshJobs, pushToast, onError]);

  // The user-initiated update check speaks BOTH ways (#80's boot check stays
  // silent on failure; a command the user ran must answer).
  const checkUpdatesNow = useCallback(() => {
    checkForUpdate()
      .then((info) =>
        pushToast(
          "ok",
          info
            ? APP_COPY.updateAvailable.replace("{version}", info.version)
            : APP_COPY.upToDate,
        ),
      )
      .catch((e) => pushToast("error", humanizeError(`update check failed: ${e}`).title));
  }, [pushToast]);

  const commands = useMemo<Command[]>(
    () => [
      { id: "new-upload", label: "New upload…", group: "Actions", keys: [modGlyph(), "N"], run: focusNewUpload },
      { id: "pause-all", label: "Pause all active uploads", group: "Actions", run: pauseAllActive },
      { id: "resume-all", label: "Resume all paused uploads", group: "Actions", run: resumeAllPaused },
      { id: "check-updates", label: "Check for updates", group: "Actions", run: checkUpdatesNow },
      { id: "goto-all", label: "All uploads", group: "Go to", keys: [modGlyph(), "1"], run: () => setFilter("all") },
      { id: "connect", label: "Connect a destination…", group: "Go to", keys: [modGlyph(), "3"], run: () => setSettingsOpen(true) },
      { id: "settings", label: "Open Settings", group: "Go to", keys: [modGlyph(), ","], run: () => setSettingsOpen(true) },
    ],
    [focusNewUpload, pauseAllActive, resumeAllPaused, checkUpdatesNow],
  );

  // The global reflexes (#91): ⌘K toggles the palette, ⌘N and ⌘, are the two
  // standard desktop shortcuts that work with the palette closed too. Window
  // listener — while IUM is focused, these never fall through to a web
  // reflex; Tauri owns the window.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        setPaletteOpen((open) => !open);
      } else if (key === "n") {
        e.preventDefault();
        setPaletteOpen(false);
        focusNewUpload();
      } else if (key === ",") {
        e.preventDefault();
        setPaletteOpen(false);
        setSettingsOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [focusNewUpload]);

  const views: JobView[] = jobs
    .filter((r) => inFilter(filter, r.state))
    .map((record) => ({
      record,
      confirmed: progress[record.id]?.confirmed ?? 0,
      speed: progress[record.id]?.speed ?? 0,
    }));

  const count = (f: Filter) => jobs.filter((r) => inFilter(f, r.state)).length;
  const activeCount = count("active");
  const queuedCount = count("queued");
  const failedCount = count("failed");
  const engineWord =
    engine.kind === "ready" ? "Connected" : engine.kind === "connecting" ? "Connecting…" : "Engine unreachable";
  const version = engine.kind === "ready" ? parseVersion(engine.detail) : null;

  return (
    <div className="app">
      <header className="toolbar">
        {/* Commands only — the ladder's Chrome rung is none: no brand block
            lives in the working chrome (pinned by anatomy.test.tsx). */}
        <button className="btn btn--primary btn--sm" onClick={focusNewUpload}>
          <IconUpload size={14} />
          New Upload
        </button>
        <span className="tb-sep" aria-hidden="true" />
        <span className="spacer" />
        {/* The ⌘K entry point (#91) — the chip teaches the shortcut, the
            palette does the work. A real button, platform-true glyph. */}
        <button
          className="btn btn--ghost cmdk-chip"
          aria-label="Commands — open the command palette"
          title="Commands"
          onClick={() => setPaletteOpen(true)}
        >
          <span aria-hidden="true">Commands</span>
          <span className="keycap" aria-hidden="true">{modGlyph()}K</span>
        </button>
        <ThemeToggle value={themePref} onChange={setThemePref} />
        <span className="tb-sep" aria-hidden="true" />
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

      <div className="app-body">
        <nav className="sidebar" aria-label="Library">
          <div className="slabel">Library</div>
          {FILTERS.map(({ key, label }) => (
            <button
              key={key}
              className={`sitem${filter === key ? " active" : ""}`}
              aria-pressed={filter === key}
              onClick={() => setFilter(key)}
            >
              <span className="sitem-icon">
                {key === "all" ? (
                  <IconUpload size={15} />
                ) : key === "active" ? (
                  <IconPlay size={15} />
                ) : key === "queued" ? (
                  <IconAlert size={15} />
                ) : key === "completed" ? (
                  <IconCheck size={15} />
                ) : (
                  <IconX size={15} />
                )}
              </span>
              <span className={key === "failed" ? "sitem-label sitem-label--fail" : "sitem-label"}>
                {label}
              </span>
              <span className="scount">{count(key)}</span>
            </button>
          ))}

          <div className="slabel">Destinations</div>
          {lanes.map((lane) => (
            <div
              key={lane.adapter}
              className="sitem"
              title={`${lane.label} — ${lane.connected ? "connected" : "not connected"}`}
            >
              <span className="sitem-icon">
                <IconCloud size={15} />
              </span>
              <span className="sitem-label">{lane.label}</span>
              {/* The dot is decoration; the STATE is text — a visually-hidden
                  word, never an aria-label on a roleless span (axe:
                  aria-prohibited-attr). */}
              <span className="sr-only">{lane.connected ? "connected" : "not connected"}</span>
              <span className={lane.connected ? "sdot sdot--on" : "sdot sdot--off"} aria-hidden="true" />
            </div>
          ))}
          <button className="sitem sadd" onClick={() => setSettingsOpen(true)}>
            <span className="sitem-icon">
              <IconPlus size={15} />
            </span>
            <span className="sitem-label">Connect destination…</span>
          </button>
        </nav>

        <main className="content">
          <CreateJobForm
            onCreated={() => {
              refreshJobs();
              pushToast("ok", APP_COPY.created);
            }}
            recents={recentEndpoints(jobs)}
            onError={onError}
          />

          {views.length === 0 ? (
            <div className="empty">
              <IconUpload size={26} />
              <div className="empty-title">
                {filter === "all" ? APP_COPY.emptyTitle : `No ${filter} uploads`}
              </div>
              <div className="empty-hint">{APP_COPY.emptyHint}</div>
            </div>
          ) : (
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
          )}
        </main>
      </div>

      {/* The docked inspector: it PUSHES the window, it never overlays —
          a flex child between the work pane and the status bar. */}
      <InspectorDrawer snapshot={inspected} onClose={() => setSelected(null)} />

      <footer className="statusbar">
        <span className="mono">
          {activeCount > 0 || queuedCount > 0 || failedCount > 0
            ? [
                activeCount > 0 ? `${activeCount} active` : null,
                queuedCount > 0 ? `${queuedCount} queued` : null,
                failedCount > 0 ? `${failedCount} failed` : null,
              ]
                .filter(Boolean)
                .join(" · ")
            : "Idle"}
        </span>
        <span className="spacer" />
        <span
          className={`engine-pill engine-pill--${
            engine.kind === "ready" ? "ok" : engine.kind === "connecting" ? "wait" : "down"
          }`}
          title={engine.kind === "connecting" ? "connecting to the engine…" : engine.detail}
        >
          <span className="dot" />
          {engineWord}
        </span>
        {version && <span className="mono st-version">{version}</span>}
      </footer>

      <SettingsDialog
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
          refreshLanes(); // a connect that happened inside must light its dot
        }}
        onError={onError}
        onSaved={(message) => pushToast("ok", message)}
        engineDetail={engine.kind === "connecting" ? undefined : engine.detail}
      />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        commands={commands}
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
