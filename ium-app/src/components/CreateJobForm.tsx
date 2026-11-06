/**
 * Create-job intake (#16, #65 Lane 1): the zero-touch front door.
 *
 * Three ways in, one code path: drop a file (Tauri drag-drop), browse
 * (official dialog plugin), or paste a path (#35 seam — the WebDriver
 * keyboard lane; a GTK file dialog is not scriptable, a text field is).
 * The shell stats whatever lands, the engine plans — the UI never computes
 * transfer truth.
 *
 * e2e contract (pinned on #65): ids `upload-path` / `upload-endpoint` /
 * `upload-adapter` / `upload-connections` / `upload-submit`; the adapter
 * select's option VALUES stay the engine registry names.
 */
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api } from "../api";
import { friendlyAdapter, humanBytes } from "../logic";
import { humanizeError } from "../copy";
import { IconAlert, IconCheck, IconFile, IconUpload, IconX } from "./icons";
import { Button, Field, Select, TextInput } from "./ui";

export interface CreateJobFormProps {
  onCreated: () => void;
  /** Journal-derived recent destinations for the datalist (presentation math). */
  recents: string[];
  /** Surface async failures as toasts — silent failures are a dev-tool habit. */
  onError: (message: string) => void;
}

export function CreateJobForm({ onCreated, recents, onError }: CreateJobFormProps) {
  const [adapters, setAdapters] = useState<string[]>([]);
  const [adapter, setAdapter] = useState("");
  const [sourcePath, setSourcePath] = useState("");
  const [sourceSize, setSourceSize] = useState<number | null>(null);
  const [endpoint, setEndpoint] = useState("");
  const [maxConns, setMaxConns] = useState(3);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [dragOver, setDragOver] = useState(false);

  useEffect(() => {
    api
      .listAdapters()
      .then((names) => {
        setAdapters(names);
        setAdapter((cur) => cur || names[0] || "");
      })
      .catch((e) => {
        const msg = `registry unavailable: ${e}`;
        setError(msg);
        onError(msg);
      });
  }, [onError]);

  // Zero-touch intake (#65): the OS drag-drop lands here and rides the same
  // stat-and-plan path as every other entry. Outside a Tauri runtime (vite
  // dev in a plain browser) getCurrentWebview() THROWS synchronously — guard
  // so the form degrades to picker + paste-a-path instead of blanking.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let alive = true;
    let cleanup: (() => void) | undefined;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            setDragOver(true);
          } else if (event.payload.type === "leave") {
            setDragOver(false);
          } else if (event.payload.type === "drop") {
            setDragOver(false);
            const path = event.payload.paths[0];
            if (typeof path === "string") void onPathTyped(path);
          }
        })
        .then((fn) => {
          if (alive) unlisten = fn;
          else fn();
        })
        .catch(() => {
          /* no Tauri runtime — degrade quietly */
        });
      cleanup = () => {
        alive = false;
        unlisten?.();
      };
    } catch {
      /* no Tauri runtime — degrade quietly */
    }
    return () => cleanup?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function pickFile() {
    try {
      const path = await open({ multiple: false, directory: false });
      if (typeof path !== "string") return;
      await onPathTyped(path);
    } catch (e) {
      setError(`file picker failed: ${e}`);
    }
  }

  async function stat(path: string): Promise<number | null> {
    try {
      const size = await api.statFile(path);
      setSourceSize(size);
      return size;
    } catch (e) {
      setSourceSize(null);
      const msg = `cannot stat file: ${e}`;
      setError(msg);
      return null;
    }
  }

  /** Paste-a-path entry (#35): the keyboard is a first-class way in. */
  async function onPathTyped(path: string) {
    setSourcePath(path);
    setError(null);
    if (!path.trim()) {
      setSourceSize(null);
      return;
    }
    await stat(path);
  }

  async function submit() {
    if (!adapter || !sourcePath || sourceSize === null || !endpoint.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await api.createJob(
        {
          source_path: sourcePath,
          source_size: sourceSize,
          adapter,
          endpoint: endpoint.trim(),
        },
        maxConns,
      );
      setSourcePath("");
      setSourceSize(null);
      setEndpoint("");
      onCreated();
    } catch (e) {
      const msg = String(e);
      setError(msg);
      onError(msg);
    } finally {
      setBusy(false);
    }
  }

  const ready = Boolean(adapter && sourcePath && sourceSize !== null && endpoint.trim() && !busy);
  const filled = Boolean(sourcePath && sourceSize !== null);
  // The copy table (#70): the human sentence + the hint, and the engine's
  // own words stay visible — the product never hides what the engine said.
  const copy = error ? humanizeError(error) : null;

  return (
    <section className="card" aria-label="New upload">
      {/* The drop zone: click = browse, drag = OS intake. Not a form control,
          so it never intersects the e2e send_keys seams. */}
      <div
        className={`dropzone${dragOver ? " dropzone--over" : ""}${filled ? " dropzone--filled" : ""}`}
        role="button"
        tabIndex={0}
        aria-label="Choose a file to upload — drop it here, browse, or paste a path below"
        onClick={() => {
          if (!filled) void pickFile();
        }}
        onKeyDown={(e) => {
          if ((e.key === "Enter" || e.key === " ") && !filled) {
            e.preventDefault();
            void pickFile();
          }
        }}
      >
        {filled ? (
          <div className="file-row">
            <IconFile className="file-icon" size={20} />
            <div style={{ minWidth: 0 }}>
              <div className="file-name">{fileName(sourcePath)}</div>
              <div className="file-meta">
                {humanBytes(sourceSize ?? 0)} · ready
              </div>
            </div>
            <IconCheck size={18} style={{ marginLeft: "auto", color: "var(--ok)" }} />
            <Button
              variant="ghost"
              size="sm"
              className="file-clear"
              aria-label="Clear selected file"
              onClick={(e) => {
                e.stopPropagation();
                setSourcePath("");
                setSourceSize(null);
                setError(null);
              }}
            >
              <IconX size={13} />
            </Button>
          </div>
        ) : (
          <>
            <IconUpload className="dropzone-icon" size={26} />
            <div className="dropzone-title">Drop a file here</div>
            <div className="dropzone-hint">
              or click to browse — any destination, any size, resumable by design
            </div>
          </>
        )}
      </div>

      <div className="form-grid">
        <Field label="Path" htmlFor="upload-path" full>
          <TextInput
            id="upload-path"
            mono
            placeholder="or paste a source path…"
            value={sourcePath}
            onChange={(e) => void onPathTyped(e.target.value)}
          />
        </Field>

        <Field label="Destination" htmlFor="upload-endpoint">
          <TextInput
            id="upload-endpoint"
            mono
            placeholder="endpoint URL"
            value={endpoint}
            list="endpoint-recents"
            onChange={(e) => setEndpoint(e.target.value)}
          />
          <datalist id="endpoint-recents">
            {recents.map((ep) => (
              <option key={ep} value={ep} />
            ))}
          </datalist>
        </Field>

        <Field label="Protocol" htmlFor="upload-adapter">
          {/* Option VALUES stay the engine registry names — the #65 seam. */}
          <Select id="upload-adapter" value={adapter} onChange={(e) => setAdapter(e.target.value)}>
            {adapters.map((a) => (
              <option key={a} value={a}>
                {friendlyAdapter(a)}
              </option>
            ))}
          </Select>
        </Field>

        <Field label="Connections" htmlFor="upload-connections">
          <TextInput
            id="upload-connections"
            type="number"
            min={1}
            max={32}
            value={maxConns}
            onChange={(e) => setMaxConns(Math.max(1, Number(e.target.value) || 1))}
          />
        </Field>

        <Button id="upload-submit" onClick={submit} disabled={!ready}>
          {busy ? "Creating…" : "Upload"}
        </Button>
      </div>

      {copy && (
        <div className="alert" role="alert">
          <IconAlert size={14} />
          <div style={{ minWidth: 0 }}>
            <div>{copy.title}</div>
            {copy.hint && <div className="alert-hint">{copy.hint}</div>}
            <code className="alert-raw">the engine said: {copy.raw}</code>
          </div>
        </div>
      )}
    </section>
  );
}

function fileName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i >= 0 ? path.slice(i + 1) : path;
}
