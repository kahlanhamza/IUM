/**
 * Create-job dialog (spec §8, issue #16). The file picker is the official
 * dialog plugin; the size comes from the shell's `stat_file` command — the
 * shell stats, the engine plans. The adapter select reflects the engine's
 * registry (`list_adapters`), never a hard-coded protocol list.
 */
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { humanBytes } from "../logic";

export interface CreateJobFormProps {
  onCreated: () => void;
}

export function CreateJobForm({ onCreated }: CreateJobFormProps) {
  const [adapters, setAdapters] = useState<string[]>([]);
  const [adapter, setAdapter] = useState("");
  const [sourcePath, setSourcePath] = useState("");
  const [sourceSize, setSourceSize] = useState<number | null>(null);
  const [endpoint, setEndpoint] = useState("");
  const [maxConns, setMaxConns] = useState(3);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .listAdapters()
      .then((names) => {
        setAdapters(names);
        setAdapter((cur) => cur || names[0] || "");
      })
      .catch((e) => setError(`registry unavailable: ${e}`));
  }, []);

  async function pickFile() {
    const path = await open({ multiple: false, directory: false });
    if (typeof path !== "string") return;
    setSourcePath(path);
    setError(null);
    try {
      setSourceSize(await api.statFile(path));
    } catch (e) {
      setSourceSize(null);
      setError(`cannot stat file: ${e}`);
    }
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
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const ready = Boolean(adapter && sourcePath && sourceSize !== null && endpoint.trim() && !busy);

  return (
    <fieldset style={fieldset}>
      <legend>New upload</legend>
      <div style={row}>
        <button onClick={pickFile}>Choose file…</button>
        <span style={mono}>{sourcePath || "no file selected"}</span>
        {sourceSize !== null && <span>({humanBytes(sourceSize)})</span>}
      </div>
      <div style={row}>
        <label>
          Destination{" "}
          <input
            style={input}
            placeholder="endpoint URL (bucket URL / tus endpoint / upload URL)"
            value={endpoint}
            onChange={(e) => setEndpoint(e.target.value)}
          />
        </label>
        <label>
          Adapter{" "}
          <select value={adapter} onChange={(e) => setAdapter(e.target.value)}>
            {adapters.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
        <label>
          Connections{" "}
          <input
            type="number"
            min={1}
            max={32}
            value={maxConns}
            onChange={(e) => setMaxConns(Math.max(1, Number(e.target.value) || 1))}
          />
        </label>
        <button onClick={submit} disabled={!ready}>
          Upload
        </button>
      </div>
      {error && <p style={{ color: "#b3261e", margin: "4px 0 0" }}>{error}</p>}
    </fieldset>
  );
}

const fieldset: React.CSSProperties = {
  border: "1px solid #d0d4cf",
  borderRadius: 8,
  padding: "8px 12px",
  marginBottom: 16,
};

const row: React.CSSProperties = {
  display: "flex",
  gap: 12,
  alignItems: "center",
  flexWrap: "wrap",
  margin: "6px 0",
};

const input: React.CSSProperties = { width: 380 };

const mono: React.CSSProperties = { fontFamily: "ui-monospace, monospace", fontSize: 12 };
