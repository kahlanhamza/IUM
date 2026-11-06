/**
 * The settings dialog (#66 Lane 2, #69): the credentials vault as a product
 * surface. Built from the #67 primitives; audited by the a11y gate.
 *
 * The secret rule is enforced by construction: the IPC view never carries
 * secret values (`present` travels, bytes never leave the shell process),
 * and only explicit drafts are saved (settings.ts changedFields). The
 * honest cost of a save is printed in the footer: credentials are read at
 * startup — changes take effect after IUM restarts.
 *
 * e2e seam: the header gear button is `#app-settings` (in the e2e locator
 * inventory); this dialog itself is not driven by the lane yet.
 */
import { useCallback, useEffect, useState } from "react";
import { api, type AdapterSchema } from "../api";
import { canSave, changedFields, toRows, type FieldRow } from "../settings";
import { IconAlert, IconCheck, IconX } from "./icons";
import { Button, Chip, cx, TextInput } from "./ui";

export interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
  /** Surfaced as toasts by the shell — silent failures are a dev-tool habit. */
  onError: (message: string) => void;
  onSaved: (message: string) => void;
}

interface AdapterSection {
  schema: AdapterSchema;
  rows: FieldRow[];
}

export function SettingsDialog({ open, onClose, onError, onSaved }: SettingsDialogProps) {
  const [sections, setSections] = useState<AdapterSection[]>([]);
  const [busy, setBusy] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    Promise.all([api.credentialsSchema(), api.credentialsGet()])
      .then(([schema, views]) => {
        const rows = toRows(views);
        setSections(
          schema.map((s) => ({ schema: s, rows: rows[s.adapter] ?? [] })),
        );
      })
      .catch((e) => onError(`settings unavailable: ${e}`));
  }, [open, onError]);

  // Escape closes — the same desktop-app reflex as the inspector.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const setDraft = useCallback((adapter: string, field: string, draft: string | null) => {
    setSections((cur) =>
      cur.map((s) =>
        s.schema.adapter !== adapter
          ? s
          : { ...s, rows: s.rows.map((r) => (r.name === field ? { ...r, draft } : r)) },
      ),
    );
  }, []);

  async function save(section: AdapterSection) {
    const fields = changedFields(section.rows);
    if (Object.keys(fields).length === 0) return;
    setBusy(true);
    try {
      await api.credentialsSet(section.schema.adapter, fields);
      onSaved(
        `${section.schema.label} credentials saved — they take effect the next time IUM starts.`,
      );
      // Re-pull the masked views so the saved chips tell the truth.
      const views = await api.credentialsGet();
      const rows = toRows(views);
      setSections((cur) =>
        cur.map((s) => ({ ...s, rows: rows[s.schema.adapter] ?? s.rows })),
      );
    } catch (e) {
      onError(`saving ${section.schema.label} failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function remove(section: AdapterSection) {
    if (confirmRemove !== section.schema.adapter) {
      setConfirmRemove(section.schema.adapter);
      return;
    }
    setConfirmRemove(null);
    setBusy(true);
    try {
      await api.credentialsDelete(section.schema.adapter);
      onSaved(`${section.schema.label} credentials removed — takes effect after restart.`);
      setSections((cur) =>
        cur.map((s) => ({
          ...s,
          rows: s.schema.adapter === section.schema.adapter
            ? s.rows.map((r) => ({ ...r, draft: "", present: false, saved: false, value: null }))
            : s.rows,
        })),
      );
    } catch (e) {
      onError(`removing ${section.schema.label} failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  if (!open) return null;

  return (
    <>
      <div className="drawer-backdrop" onClick={onClose} aria-hidden="true" />
      <div className="drawer" role="dialog" aria-modal="true" aria-label="Settings — adapter credentials">
        <div className="drawer-head">
          <div style={{ minWidth: 0 }}>
            <h2 className="drawer-title">Settings</h2>
            <div className="drawer-sub">Credentials the engine reads at startup</div>
          </div>
          <Button variant="ghost" size="icon" aria-label="Close settings" onClick={onClose}>
            <IconX size={14} />
          </Button>
        </div>

        {sections.map((s) => {
          const savable = canSave(s.rows) && Object.keys(changedFields(s.rows)).length > 0;
          return (
            <section key={s.schema.adapter} className="card" aria-label={s.schema.label}>
              <div className="section-head" style={{ marginBottom: 8 }}>
                <h3 className="drawer-h3" style={{ margin: 0 }}>{s.schema.label}</h3>
              </div>
              <p className="settings-note">{s.schema.note}</p>
              <div className="form-grid">
                {s.rows.map((r) => (
                  <SecretField
                    key={r.name}
                    section={s.schema.adapter}
                    row={r}
                    busy={busy}
                    onDraft={(d) => setDraft(s.schema.adapter, r.name, d)}
                  />
                ))}
              </div>
              <div className="cell-actions" style={{ justifyContent: "flex-end", marginTop: 4 }}>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy || s.rows.every((r) => !r.saved && r.draft === null)}
                  onClick={() => void remove(s)}
                >
                  {confirmRemove === s.schema.adapter ? "Click again to remove" : "Remove saved"}
                </Button>
                <Button
                  disabled={!savable || busy}
                  onClick={() => void save(s)}
                  aria-label={`Save ${s.schema.label} credentials`}
                >
                  Save
                </Button>
              </div>
            </section>
          );
        })}

        <p className="settings-footer" role="note">
          <IconAlert size={13} />
          Credentials are read when IUM starts — changes take effect the next time it starts.
          A real environment variable always wins over a saved value.
        </p>
      </div>
    </>
  );
}

interface SecretFieldProps {
  section: string;
  row: FieldRow;
  busy: boolean;
  onDraft: (draft: string | null) => void;
}

function SecretField({ section, row, busy, onDraft }: SecretFieldProps) {
  const [reveal, setReveal] = useState(false);
  const clearing = row.draft === "";
  const id = `cred-${section}-${row.name}`;
  return (
    <div className={cx("field", row.name === "extra_env" && "field--full")}>
      <label className="field-label" htmlFor={id}>
        {row.label}
        {row.required ? " *" : ""}
      </label>
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <TextInput
          id={id}
          type={row.secret && !reveal ? "password" : "text"}
          value={row.draft ?? (row.secret ? "" : (row.value ?? ""))}
          placeholder={
            row.saved
              ? "saved — type to replace"
              : row.secret
                ? "enter a value"
                : "not set"
          }
          autoComplete="off"
          disabled={busy}
          onChange={(e) => onDraft(e.target.value)}
          style={clearing ? { borderColor: "var(--warn)" } : undefined}
        />
        {row.secret && (
          <Button
            variant="ghost"
            size="sm"
            aria-label={reveal ? `Hide ${row.label}` : `Reveal ${row.label}`}
            disabled={busy}
            onClick={() => setReveal((v) => !v)}
          >
            {reveal ? "Hide" : "Show"}
          </Button>
        )}
        {row.secret && row.saved && row.draft === null && <Chip state="saved">saved</Chip>}
        {clearing && <Chip state="canceled">will be removed</Chip>}
        {row.draft !== null && row.draft !== "" && (
          <IconCheck size={14} style={{ color: "var(--ok)", flex: "none" }} />
        )}
      </div>
    </div>
  );
}
