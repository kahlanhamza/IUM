/**
 * The settings dialog as the connections surface (#66 Lane 2, #69, #81,
 * #82). Built from the #67 primitives; audited by the a11y gate.
 *
 * The secret rule is enforced by construction: the IPC view never carries
 * secret values (`present` travels, bytes never leave the shell process),
 * and only explicit drafts are saved (settings.ts changedFields). #81 made
 * saves live (apply_credentials — no restart) and gave every lane a
 * read-only Test connection.
 *
 * #82 is the owner's "Google authentication style" bar: each lane
 * collapses to one honest row — its state sentence and ONE action — and
 * Connect opens a focused single-lane sheet whose success CLOSES the
 * drawer (the window closes and the app is connected). The rclone bridge
 * lane leads with the browser sign-in: the shell spawns `rclone authorize
 * <backend>`, the provider's real page opens in the user's browser, they
 * sign in as normal, and the token merges INSIDE the shell (a masked
 * secret value never crosses the UI). Generic protocols keep the focused
 * sheet — honest by capability, per the owner's own framing.
 *
 * e2e seam: the header gear button is `#app-settings` (in the e2e locator
 * inventory); this dialog itself is not driven by the lane yet.
 */
import { useCallback, useEffect, useState } from "react";
import { api, type AdapterSchema, type ProbeResult, type TelemetryStatus } from "../api";
import { canSave, changedFields, probeFields, toRows, type FieldRow } from "../settings";
import { connectionState, OAUTH_BACKENDS } from "../connect";
import { APP_COPY, NOTIFICATIONS_COPY, TELEMETRY_COPY } from "../copy";
import { useNotifyPref } from "../notify";
import { probeCopy, probeFailCopy, type ProbeState } from "../auth";
import { IconAlert, IconCheck, IconX, Logomark } from "./icons";
import { Button, Chip, cx, Select, TextInput } from "./ui";
import { ThemeToggle } from "./ThemeToggle";
import { useThemePref } from "../useTheme";
import {
  checkForUpdate,
  downloadAndInstall,
  restartToUpdate,
  updateSupported,
  type DownloadProgress,
} from "../updater";

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
  /** The focused connect sheet (#82): which adapter's fields are open —
   *  null = the one-click list. */
  const [expanded, setExpanded] = useState<string | null>(null);
  /** Per-adapter Test-connection state (#81): one entry per schema id. */
  const [probe, setProbe] = useState<Record<string, { state: ProbeState; result?: ProbeResult }>>({});
  /** Where the secret values actually live (the footer's honesty marker). */
  const [secrets, setSecrets] = useState<"keyring" | "file" | null>(null);

  useEffect(() => {
    if (!open) return;
    api
      .secretMode()
      .then(setSecrets)
      .catch(() => setSecrets(null)); // the footer then stays neutral
  }, [open]);

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

  // Escape closes — with a sheet open it steps back first (#82's reflex).
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (expanded !== null) setExpanded(null);
        else onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, expanded, onClose]);

  const setDraft = useCallback((adapter: string, field: string, draft: string | null) => {
    setSections((cur) =>
      cur.map((s) =>
        s.schema.adapter !== adapter
          ? s
          : { ...s, rows: s.rows.map((r) => (r.name === field ? { ...r, draft } : r)) },
      ),
    );
  }, []);

  /** Re-pull the masked views so the chips and the connect rows' state
   *  sentences tell the truth after any vault change. */
  const refreshViews = useCallback(async () => {
    const views = await api.credentialsGet();
    const rows = toRows(views);
    setSections((cur) =>
      cur.map((s) => ({ ...s, rows: rows[s.schema.adapter] ?? s.rows })),
    );
  }, []);

  async function remove(section: AdapterSection) {
    if (confirmRemove !== section.schema.adapter) {
      setConfirmRemove(section.schema.adapter);
      return;
    }
    setConfirmRemove(null);
    setBusy(true);
    try {
      await api.credentialsDelete(section.schema.adapter);
      // No credentials = no lane (the registry reflects the vault, #81).
      await api.applyCredentials(section.schema.adapter);
      onSaved(APP_COPY.authRemovedLive);
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

  /** The read-only credential check (#81): stored values overridden by
   *  drafts — Test tests what is actually saved, drafts what would be.
   *  Returns the outcome so the connect flow can act on it (#82). */
  async function testConnection(section: AdapterSection): Promise<ProbeResult | null> {
    const id = section.schema.adapter;
    setProbe((cur) => ({ ...cur, [id]: { state: "testing" as ProbeState } }));
    try {
      const result = await api.probeCredentials(id, probeFields(section.rows));
      const state: ProbeState = !result.checked
        ? "unchecked"
        : result.ok
          ? "ok"
          : "fail";
      setProbe((cur) => ({ ...cur, [id]: { state, result } }));
      return result;
    } catch (e) {
      setProbe((cur) => ({
        ...cur,
        [id]: {
          state: "fail" as ProbeState,
          result: { checked: true, ok: false, sentence: String(e), final: true },
        },
      }));
      return null;
    }
  }

  /** The connect action (#82): save what changed, hot-apply, then answer
   *  through the probe with the typed drafts still in hand (secrets never
   *  travel back from the vault, so the probe rides the drafts). A
   *  connected or honest-unchecked answer CLOSES the drawer — the "the
   *  window closes and the app is connected" reflex; a failure keeps the
   *  sheet open with the sentence in view. */
  async function saveAndConnect(section: AdapterSection) {
    const changed = changedFields(section.rows);
    if (Object.keys(changed).length === 0) {
      if (section.rows.every((r) => !r.required || r.present)) {
        // Already saved and untouched — the lane is live; say so and close.
        onSaved(APP_COPY.connectDone);
        onClose();
      }
      return;
    }
    setBusy(true);
    try {
      await api.credentialsSet(section.schema.adapter, changed);
      await api.applyCredentials(section.schema.adapter);
      const result = await testConnection(section);
      if (result && (!result.checked || result.ok)) {
        onSaved(APP_COPY.connectDone);
        onClose();
      }
      await refreshViews();
    } catch (e) {
      onError(`connecting ${section.schema.label} failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  if (!open) return null;

  const expandedSection = expanded
    ? (sections.find((s) => s.schema.adapter === expanded) ?? null)
    : null;

  return (
    <>
      {/* #88: the centered 640 px modal over the smoke scrim — the drawer
          chrome is retired; the internals (the lane list, the connect
          sheets, the Esc ladder) are untouched. The shadow rides ONLY
          here (the shadow law). */}
      <div className="scrim" onClick={onClose} aria-hidden="true" />
      <div className="modal" role="dialog" aria-modal="true" aria-label="Settings — connections">
        <div className="mhead">
          <div style={{ minWidth: 0 }}>
            <h2 className="mtitle">Settings</h2>
            <div className="msub">
              {expandedSection ? expandedSection.schema.label : "Connections the engine reads live"}
            </div>
          </div>
          <Button variant="ghost" size="icon" aria-label="Close settings" onClick={onClose}>
            <IconX size={14} />
          </Button>
        </div>

        <AppearanceSection />

        <NotifySection />

        <PrivacySection onError={onError} onSaved={onSaved} />

        {expandedSection === null ? (
          sections.map((s) => {
            const state = connectionState(s.rows);
            return (
              <section key={s.schema.adapter} className="card" aria-label={s.schema.label}>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 10,
                    justifyContent: "space-between",
                  }}
                >
                  <div style={{ minWidth: 0 }}>
                    <h3 className="drawer-h3" style={{ margin: 0 }}>{s.schema.label}</h3>
                    <div style={{ fontSize: 12, color: "var(--text-2)" }}>
                      {state === "connected"
                        ? APP_COPY.connectStateConnected
                        : state === "partial"
                          ? APP_COPY.connectStatePartial
                          : APP_COPY.connectStateEmpty}
                    </div>
                  </div>
                  <Button
                    variant={state === "connected" ? "ghost" : "primary"}
                    size="sm"
                    aria-label={`${state === "connected" ? "Details for" : "Connect"} ${s.schema.label}`}
                    onClick={() => setExpanded(s.schema.adapter)}
                  >
                    {state === "connected" ? "Details" : "Connect"}
                  </Button>
                </div>
                <p className="settings-note" style={{ margin: "6px 0 0" }}>{s.schema.note}</p>
              </section>
            );
          })
        ) : (
          <ConnectSheet
            section={expandedSection}
            busy={busy}
            probe={probe[expandedSection.schema.adapter]}
            confirmRemove={confirmRemove === expandedSection.schema.adapter}
            onBack={() => setExpanded(null)}
            onDraft={setDraft}
            onTest={() => void testConnection(expandedSection)}
            onRemove={() => void remove(expandedSection)}
            onConnect={() => void saveAndConnect(expandedSection)}
            onError={onError}
            onConnected={(message) => {
              onSaved(message);
              onClose();
            }}
            onVaultChanged={() => refreshViews()}
          />
        )}

        <p className="settings-footer" role="note">
          <IconAlert size={13} />
          {secrets === "keyring"
            ? APP_COPY.secretsKeychain
            : secrets === "file"
              ? APP_COPY.secretsFileFallback
              : "Secrets are stored locally; their exact home is reported once the shell answers. "}
          {APP_COPY.authFooterEnv} {APP_COPY.authFooterLive}
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
          <UpdateCard />
        </div>
      </div>
    </>
  );
}

/**
 * The focused connect sheet (#82): ONE lane at a time — a back control,
 * the browser sign-in when the lane offers one (the rclone bridge), the
 * lane's own fields, the Test control, and one connect action. The
 * probe's answer renders in the sheet: an honest sentence, never a code.
 */
interface ConnectSheetProps {
  section: AdapterSection;
  busy: boolean;
  probe?: { state: ProbeState; result?: ProbeResult };
  confirmRemove: boolean;
  onBack: () => void;
  onDraft: (adapter: string, field: string, draft: string | null) => void;
  onTest: () => void;
  onRemove: () => void;
  onConnect: () => void;
  onError: (message: string) => void;
  onConnected: (message: string) => void;
  onVaultChanged: () => Promise<void>;
}

function ConnectSheet({
  section,
  busy,
  probe: p,
  confirmRemove,
  onBack,
  onDraft,
  onTest,
  onRemove,
  onConnect,
  onError,
  onConnected,
  onVaultChanged,
}: ConnectSheetProps) {
  const { schema, rows } = section;
  const isBridge = schema.adapter === "rclone_bridge";
  // The first-class OAuth lane (#84): its sheet is the sign-in itself —
  // no picker, no fields (the token row is hidden; its presence is the
  // state). The drawer closes itself on a green answer (the reflex).
  const driveLock = schema.adapter === "google_drive" ? "drive" : undefined;
  // The YouTube lane (#85): its sheet is the two one-time client fields
  // (visible) + the loopback Google sign-in. The hidden token row's
  // presence is the state.
  const isYouTube = schema.adapter === "youtube";
  const visibleRows = rows.filter((r) => !r.hidden);
  const binDraft = rows.find((r) => r.name === "bin")?.draft ?? null;
  return (
    <section className="card" aria-label={schema.label}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <Button variant="ghost" size="sm" onClick={onBack} aria-label="Back to all connections">
          ← All connections
        </Button>
      </div>
      <h3 className="drawer-h3" style={{ margin: "8px 0 0" }}>{schema.label}</h3>
      <p className="settings-note">{schema.note}</p>
      {isBridge && (
        <BrowserSignIn
          binDraft={binDraft}
          busy={busy}
          onError={onError}
          onConnected={onConnected}
          onVaultChanged={onVaultChanged}
        />
      )}
      {driveLock && (
        <BrowserSignIn
          binDraft={binDraft}
          busy={busy}
          locked={driveLock}
          onError={onError}
          onConnected={onConnected}
          onVaultChanged={onVaultChanged}
        />
      )}
      {isYouTube && (
        <GoogleSignIn
          rows={rows}
          busy={busy}
          onError={onError}
          onConnected={onConnected}
          onVaultChanged={onVaultChanged}
        />
      )}
      <div className="form-grid">
        {visibleRows.map((r) => (
          <SecretField
            key={r.name}
            section={schema.adapter}
            row={r}
            busy={busy}
            onDraft={(d) => onDraft(schema.adapter, r.name, d)}
          />
        ))}
      </div>
      <div className="cell-actions" style={{ justifyContent: "flex-end", marginTop: 4 }}>
        <Button
          variant="ghost"
          size="sm"
          disabled={busy}
          onClick={onTest}
          aria-label={`Test ${schema.label} connection`}
        >
          {probeCopy(p?.state ?? "idle")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || rows.every((r) => !r.saved && r.draft === null)}
          onClick={onRemove}
        >
          {confirmRemove ? "Click again to remove" : "Remove saved"}
        </Button>
        <Button
          disabled={!canSave(rows) || busy}
          onClick={onConnect}
          aria-label={`Save and connect ${schema.label}`}
        >
          Save &amp; connect
        </Button>
      </div>
      {/* The probe's answer (#81): one honest line — the acceptance
          sentence, the not-checked sentence, or the copy table's verdict
          with the raw truth kept in view. */}
      {(() => {
        if (!p || p.state === "idle" || p.state === "testing") return null;
        const r = p.result;
        if (p.state === "ok") {
          return (
            <p className="settings-note" role="status">
              {probeCopy("ok", r)}
            </p>
          );
        }
        if (p.state === "unchecked") {
          return (
            <p className="settings-note" role="status">
              {probeCopy("unchecked", r)}
            </p>
          );
        }
        if (!r) return null;
        const copy = probeFailCopy(r);
        return (
          <p className="settings-note" role="alert">
            {copy.title}
            {copy.hint ? ` ${copy.hint}` : ""}
            {r.final ? "" : ` The engine said: ${copy.raw}`}
          </p>
        );
      })()}
    </section>
  );
}

/**
 * The browser sign-in (#82): the obvious OAuth platforms sign in on their
 * OWN page — the shell spawns `rclone authorize`, the user's browser
 * opens it, they sign in as normal (2FA and consent screens included),
 * and the shell captures the token, merges it into the bridge lane's
 * extra_env INSIDE the process, and hot-applies the lane. The UI never
 * sees a token byte; it only hears the outcome. The list is the allowlist;
 * DRIVE joined it when the owner lifted the shelving (#84) and the
 * first-class Google Drive lane signs in with a LOCKED backend — no picker.
 */
function BrowserSignIn({
  binDraft,
  busy,
  locked,
  onError,
  onConnected,
  onVaultChanged,
}: {
  binDraft: string | null;
  busy: boolean;
  /** Fixed backend (#84): the first-class lanes' sheets render NO picker —
   *  the Google Drive sheet signs in with Google, full stop. */
  locked?: string;
  onError: (message: string) => void;
  onConnected: (message: string) => void;
  onVaultChanged: () => Promise<void>;
}) {
  const [picked, setPicked] = useState(OAUTH_BACKENDS[0].id);
  const [waiting, setWaiting] = useState(false);
  const backend = locked ?? picked;
  const chosen = OAUTH_BACKENDS.find((b) => b.id === backend) ?? OAUTH_BACKENDS[0];

  async function connect() {
    setWaiting(true);
    try {
      await api.authorizeOauth(backend, binDraft ?? undefined);
      await onVaultChanged();
      onConnected(APP_COPY.oauthDone);
    } catch (e) {
      onError(String(e));
    } finally {
      setWaiting(false);
    }
  }

  return (
    <div className="oauth-block" aria-label="Browser sign-in">
      {locked ? null : (
        <div className="field">
          <label className="field-label" htmlFor="oauth-backend">Platform</label>
          <Select
            id="oauth-backend"
            value={picked}
            disabled={busy || waiting}
            onChange={(e) => setPicked(e.target.value)}
          >
            {OAUTH_BACKENDS.map((b) => (
              <option key={b.id} value={b.id}>{b.label}</option>
            ))}
          </Select>
        </div>
      )}
      <p className="settings-note" style={{ margin: "4px 0 0" }}>{chosen.note}</p>
      <div className="cell-actions" style={{ justifyContent: "flex-start", marginTop: 4 }}>
        <Button disabled={busy || waiting} onClick={() => void connect()}>
          {waiting ? "Signing in…" : `Connect ${chosen.label}`}
        </Button>
      </div>
      {waiting && (
        <p className="settings-note" role="status" style={{ margin: "6px 0 0" }}>
          {APP_COPY.oauthWaiting}
        </p>
      )}
    </div>
  );
}

/**
 * The YouTube sign-in (#85): the lane's sheet carries the two one-time
 * client fields (visible rows, masked secret included) + this block. The
 * drafts travel to the shell (saved values win when the drafts are empty);
 * the browser opens Google's real consent screen, the redirect returns to
 * the shell's loopback listener, and the drawer closes itself on a green
 * answer. The UI never sees a token byte.
 */
function GoogleSignIn({
  rows,
  busy,
  onError,
  onConnected,
  onVaultChanged,
}: {
  rows: FieldRow[];
  busy: boolean;
  onError: (message: string) => void;
  onConnected: (message: string) => void;
  onVaultChanged: () => Promise<void>;
}) {
  const [waiting, setWaiting] = useState(false);
  const clientId = rows.find((r) => r.name === "client_id");
  const clientSecret = rows.find((r) => r.name === "client_secret");
  const ready = (r: FieldRow | undefined) =>
    !!r && ((r.draft !== null && r.draft !== "") || r.present);
  const canConnect = ready(clientId) && ready(clientSecret);

  async function connect() {
    setWaiting(true);
    try {
      await api.connectYoutube(
        clientId?.draft ?? undefined,
        clientSecret?.draft ?? undefined,
      );
      await onVaultChanged();
      onConnected(APP_COPY.oauthDone);
    } catch (e) {
      onError(String(e));
    } finally {
      setWaiting(false);
    }
  }

  return (
    <div className="oauth-block" aria-label="Google sign-in">
      <div className="cell-actions" style={{ justifyContent: "flex-start", marginTop: 4 }}>
        <Button disabled={busy || waiting || !canConnect} onClick={() => void connect()}>
          {waiting ? "Signing in…" : "Connect YouTube"}
        </Button>
      </div>
      {waiting && (
        <p className="settings-note" role="status" style={{ margin: "6px 0 0" }}>
          {APP_COPY.oauthWaiting}
        </p>
      )}
      {!canConnect && (
        <p className="settings-note" style={{ margin: "6px 0 0" }}>
          Both the client id and the client secret are needed before the sign-in can open.
        </p>
      )}
    </div>
  );
}

/**
 * The update control (#80): one card, six honest states. Check is
 * user-triggered here (the startup check lives in App and is silent);
 * an install NEVER starts without the user's click (the consent rule).
 * Copy follows the brand voice: calm sentences, no exclamation marks.
 * Rendered only when a real updater is reachable (#40 lesson — a
 * permission that was never granted is a silent denial).
 */
type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "uptodate" }
  | { kind: "available"; version: string; notes: string | null }
  | { kind: "downloading"; received: number; total?: number }
  | { kind: "ready" }
  | { kind: "error"; message: string };

function UpdateCard() {
  const [state, setState] = useState<UpdateState>({ kind: "idle" });
  const supported = updateSupported();
  if (!supported) return null;

  const checkNow = async () => {
    setState({ kind: "checking" });
    try {
      const info = await checkForUpdate();
      setState(
        info
          ? { kind: "available", version: info.version, notes: info.notes }
          : { kind: "uptodate" },
      );
    } catch (e) {
      setState({ kind: "error", message: e instanceof Error ? e.message : String(e) });
    }
  };

  const install = async () => {
    setState((s) =>
      s.kind === "available"
        ? { kind: "downloading", received: 0, total: undefined }
        : s,
    );
    try {
      await downloadAndInstall((p: DownloadProgress) => {
        setState((s) =>
          s.kind === "downloading"
            ? { kind: "downloading", received: p.downloaded, total: p.total ?? s.total }
            : s,
        );
      });
      setState({ kind: "ready" });
    } catch (e) {
      setState({ kind: "error", message: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <div className="about-update" aria-label="Update IUM">
      {state.kind === "idle" && (
        <Button variant="ghost" size="sm" onClick={checkNow}>
          Check for updates
        </Button>
      )}
      {state.kind === "checking" && <span className="about-update-line">Checking for updates…</span>}
      {state.kind === "uptodate" && (
        <span className="about-update-line">IUM {__APP_VERSION__} is up to date.</span>
      )}
      {state.kind === "available" && (
        <>
          <span className="about-update-line">IUM {state.version} is available. Restart to update when the download finishes.</span>
          <Button variant="primary" size="sm" onClick={install}>
            Download {state.version}
          </Button>
        </>
      )}
      {state.kind === "downloading" && (
        <span className="about-update-line">
          {state.total
            ? `Downloading ${Math.min(100, Math.round((state.received / state.total) * 100))}%`
            : "Downloading…"}
        </span>
      )}
      {state.kind === "ready" && (
        <>
          <span className="about-update-line">The update is ready. Restart to update.</span>
          <Button variant="primary" size="sm" onClick={() => void restartToUpdate()}>
            Restart to update
          </Button>
        </>
      )}
      {state.kind === "error" && (
        <span className="about-update-line" role="alert">
          The update check failed: {state.message}
        </span>
      )}
    </div>
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
 * The Notifications card (#93): ONE switch for the OS surface — the pure
 * transition rule in notify.ts decides when, the copy matrix decides what;
 * the user only answers "tell me or don't". Same card system as Privacy:
 * a checkbox row + the honest sentence about the focus rule.
 */
function NotifySection() {
  const [enabled, setEnabled] = useNotifyPref();
  return (
    <section className="card" aria-label="Notifications">
      <h3 className="drawer-h3" style={{ margin: "0 0 8px" }}>Notifications</h3>
      <div style={{ display: "flex", gap: 8, alignItems: "center", margin: "10px 0" }}>
        <input
          id="notify-enabled"
          type="checkbox"
          checked={enabled}
          onChange={(e) => setEnabled(e.target.checked)}
          style={{ width: 15, height: 15, accentColor: "var(--accent)", cursor: "pointer" }}
        />
        <label htmlFor="notify-enabled" style={{ cursor: "pointer" }}>
          {NOTIFICATIONS_COPY.settingsLabel}
        </label>
      </div>
      <p className="settings-note" style={{ color: "var(--text-2)" }}>
        {NOTIFICATIONS_COPY.settingsNote}
      </p>
    </section>
  );
}

/**
 * The Appearance control (#86): the two-ink rule made user-facing. System
 * is the default; an explicit Light/Dark overrides the OS until the user
 * returns the choice. The preference is a UI concern — localStorage, no
 * engine round-trip, applied to the document root immediately.
 */
function AppearanceSection() {
  // The shared store (#88): the Settings card and the toolbar's compact
  // control are ONE state — moving either moves both in the same tick.
  const [pref, setPref] = useThemePref();

  return (
    <section className="card" aria-label="Appearance">
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 10,
        }}
      >
        <div style={{ minWidth: 0 }}>
          <h3 className="drawer-h3" style={{ margin: 0 }}>Appearance</h3>
          <div style={{ fontSize: 12, color: "var(--text-2)" }}>
            Two inks — the mark's dark ink on light, its light ink on dark
          </div>
        </div>
        <ThemeToggle value={pref} onChange={setPref} />
      </div>
    </section>
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
      <h3 className="drawer-h3" style={{ margin: "0 0 8px" }}>Privacy</h3>
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
