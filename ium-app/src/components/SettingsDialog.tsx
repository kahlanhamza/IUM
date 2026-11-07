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
import { api, type AdapterSchema, type TelemetryStatus } from "../api";
import { canSave, changedFields, toRows, type FieldRow } from "../settings";
import { TELEMETRY_COPY } from "../copy";
import { IconAlert, IconCheck, IconX, Logomark } from "./icons";
import { Button, Chip, cx, TextInput } from "./ui";

export interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
  /** Surfaced as toasts by the shell — silent failures are a dev-tool habit. */
  onError: (message: string) => void;
  onSaved: (message: string) => void;
  /** The engine's own status sentence — lives HERE (#77), in About, never
   *  as permanent header chrome. The shell passes it when known. */
  engineDetail?: string;
}

interface AdapterSection {
  schema: AdapterSchema;
  rows: FieldRow[];
}

export function SettingsDialog({ open, onClose, onError, onSaved, engineDetail }: SettingsDialogProps) {
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

        <PrivacySection onError={onError} onSaved={onSaved} />

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

        {/* About (#77): the version and the engine's own words have a home —
            the header never carries them (the quiet-chrome rule). */}
        <div className="about" aria-label="About IUM">
          <Logomark size={22} />
          <div style={{ minWidth: 0 }}>
            <div className="about-name">
              IUM <span className="about-version">{__APP_VERSION__}</span>
            </div>
            <div className="about-line">
              Internet Upload Manager — reliability-engineered uploads.
            </div>
            {engineDetail && <div className="about-engine">{engineDetail}</div>}
          </div>
        </div>
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

/**
 * The Privacy card (#66 Lane 7, #75): the telemetry stance as a product
 * surface — the written stance sentence, the opt-out master switch (the
 * engine enforces it at the single write point), what is and is never
 * collected (sentences from the copy table), and the local buffer's count
 * + path + two-step clear. The card LEADS the dialog: trust first.
 */
function PrivacySection({
  onError,
  onSaved,
}: {
  onError: (message: string) => void;
  onSaved: (message: string) => void;
}) {
  const [st, setSt] = useState<TelemetryStatus>({ consent: true, count: 0, path: "" });
  const [confirmClear, setConfirmClear] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .telemetryStatus()
      .then(setSt)
      .catch((e) => onError(`privacy status unavailable: ${e}`));
  }, [onError]);

  async function toggle(on: boolean) {
    setBusy(true);
    try {
      await api.telemetrySetConsent(on);
      setSt((s) => ({ ...s, consent: on }));
      onSaved(on ? TELEMETRY_COPY.consentOnToast : TELEMETRY_COPY.consentOffToast);
    } catch (e) {
      onError(`changing the diagnostics switch failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function clear() {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    setConfirmClear(false);
    setBusy(true);
    try {
      await api.telemetryClear();
      setSt((s) => ({ ...s, count: 0 }));
      onSaved(TELEMETRY_COPY.clearedToast);
    } catch (e) {
      onError(`clearing the local diagnostics failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card" aria-label="Privacy">
      <div className="section-head" style={{ marginBottom: 8 }}>
        <h3 className="drawer-h3" style={{ margin: 0 }}>Privacy</h3>
      </div>
      <p className="settings-note">{TELEMETRY_COPY.stance}</p>
      <div style={{ display: "flex", gap: 8, alignItems: "center", margin: "10px 0" }}>
        <input
          id="telemetry-consent"
          type="checkbox"
          checked={st.consent}
          disabled={busy}
          onChange={(e) => void toggle(e.target.checked)}
          style={{ width: 15, height: 15, accentColor: "var(--accent)", cursor: "pointer" }}
        />
        <label htmlFor="telemetry-consent" style={{ cursor: "pointer" }}>
          {TELEMETRY_COPY.switchLabel}
        </label>
      </div>
      <p className="settings-note">{TELEMETRY_COPY.collectJob}</p>
      <p className="settings-note">{TELEMETRY_COPY.collectCrash}</p>
      <p className="settings-note" style={{ color: "var(--text-2)" }}>
        {TELEMETRY_COPY.neverLine}
      </p>
      <div className="cell-actions" style={{ justifyContent: "space-between", marginTop: 8 }}>
        <span className="settings-note" style={{ margin: 0 }}>
          {st.count} recorded so far. {TELEMETRY_COPY.bufferNote}
          {st.path ? (
            <>
              {" "}
              <code style={{ fontSize: 11, wordBreak: "break-all" }}>{st.path}</code>
            </>
          ) : null}
        </span>
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || st.count === 0}
          onClick={() => void clear()}
        >
          {confirmClear ? "Click again to clear" : "Clear local events"}
        </Button>
      </div>
    </section>
  );
}
