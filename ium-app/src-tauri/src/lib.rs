//! IUM desktop shell — thin Tauri 2 layer over `ium-core`.
//!
//! Rule (spec §2.3): the UI never computes transfer truth. Commands only
//! forward to the Supervisor/JobManager; the SQLite journal is the single
//! source of truth. `job_updated` events carry the same journal snapshots a
//! poll would return — the emitter READS the journal, it never derives.
//!
//! Issue #15: the shell actually uploads. `create_job` drives the job
//! through the engine's `Supervisor` (spawn/cancel/pause/resume semantics
//! live in `ium-core::supervisor`, testable without a webview); `cancel`
//! journals the honest `Canceled` terminal state BEFORE the drive task is
//! aborted.

mod auth;
mod telemetry;
mod vault;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ium_core::{
    HttpOneShotAdapter, JobManager, JobRecord, JobSnapshot, JobSpec, S3MultipartAdapter,
    S3MultipartConfig, Supervisor, TeraboxAdapter, TeraboxConfig, TransferAdapter, TusAdapter,
};
use tauri::{Emitter, Manager, State};

struct Engine(Arc<Supervisor>);

/// The telemetry pipeline (#75): one instance, reachable from the panic
/// hook (which runs outside any State) and from the watcher's terminal
/// transition. Unset = drop silently (a panic before setup is nobody's
/// telemetry problem).
static TELEMETRY: OnceLock<Arc<telemetry::Telemetry>> = OnceLock::new();

/// Registry names for the create-job form's adapter select (#16): the UI
/// reflects the engine's registry, it never hard-codes protocol lists.
#[tauri::command]
fn list_adapters(engine: State<Engine>) -> Vec<String> {
    engine
        .0
        .manager()
        .adapter_names()
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ium-session.db")
}

/// The vault lives in the OS config dir (not data — it is configuration,
/// and some backup tools treat data differently) (#69).
fn vault_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The vault constructor (#81): the OS keychain is probed once; an available
/// secret service carries every SECRET field value (the 0600 file keeps
/// non-secrets only), an unavailable one falls back to the file with the
/// honest marker. Either way the JSON records the mode the UI prints.
fn vault(app: &tauri::AppHandle) -> vault::Vault {
    vault::Vault::open_keychain(&vault_dir(app), keychain_store())
}

/// Is the OS secret service usable? Probed once per process: an entry read
/// that answers `NoEntry` proves the service is ALIVE (the keychain works,
/// nothing is stored yet); any other error means no usable service — the
/// honest file fallback.
fn keychain_available() -> bool {
    static PROBE: OnceLock<bool> = OnceLock::new();
    *PROBE.get_or_init(|| match keyring::Entry::new("ium", "__probe") {
        Ok(entry) => matches!(entry.get_password(), Ok(_) | Err(keyring::Error::NoEntry)),
        Err(_) => false,
    })
}

fn keychain_store() -> Option<Box<dyn vault::SecretStore>> {
    if keychain_available() {
        Some(Box::new(KeyringStore))
    } else {
        None
    }
}

/// The real OS keychain (spec §Secrets): one entry per `{adapter}.{field}`,
/// service "ium" — Windows Credential Manager / macOS Keychain / Linux
/// Secret Service per the keyring crate's platform features. Its compile
/// proof rides CI per the #61 contract (the sandbox has no WebKitGTK).
struct KeyringStore;

impl vault::SecretStore for KeyringStore {
    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new("ium", account)
            .map_err(|e| e.to_string())?
            .set_password(value)
            .map_err(|e| e.to_string())
    }
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new("ium", account)
            .map_err(|e| e.to_string())?
            .get_password()
        {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new("ium", account)
            .map_err(|e| e.to_string())?
            .delete_credential()
        {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

fn engine_error(e: ium_core::EngineError) -> String {
    e.to_string()
}

/// The one adapter factory (#81): boot registration and the settings UI's
/// hot apply share it. Credentials come from the process env — which the
/// vault has already filled from the settings UI (env wins: real env vars
/// are never overwritten). A lane without credentials is `None` — absence
/// is honest, never guessed.
fn build_adapter(id: &str) -> Option<Arc<dyn TransferAdapter>> {
    match id {
        "s3_multipart" => match S3MultipartConfig::from_env() {
            Ok(config) => match S3MultipartAdapter::new(config) {
                Ok(adapter) => Some(Arc::new(adapter)),
                Err(e) => {
                    tracing::warn!("s3_multipart unavailable: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("s3_multipart not configured (no env credentials): {e}");
                None
            }
        },
        "terabox" => match TeraboxConfig::from_env() {
            Ok(config) => Some(Arc::new(TeraboxAdapter::new(config))),
            Err(e) => {
                tracing::warn!("terabox not configured (no ndus cookie): {e}");
                None
            }
        },
        // Feishu Drive (#56) — core-proven since M9; the vault/env gives it a
        // product surface now (#69). FeishuAdapter::new is infallible (base is
        // a constant here) — only the env check decides the lane.
        "feishu" => match ium_core::adapter_feishu::FeishuConfig::from_env() {
            Ok(config) => Some(Arc::new(ium_core::adapter_feishu::FeishuAdapter::new(
                config,
            ))),
            Err(e) => {
                tracing::warn!("feishu not configured (no tenant creds): {e}");
                None
            }
        },
        // The YouTube lane (#85): IUM's first native OAuth adapter — the
        // config is sync (client id/secret + the stored TokenSet), no
        // session at construction. Absence is the honest no-lane state.
        "youtube" => match ium_core::YoutubeConfig::from_env() {
            Ok(cfg) => Some(Arc::new(ium_core::YoutubeAdapter::new(cfg))),
            Err(e) => {
                tracing::warn!("youtube not configured (no client/token): {e}");
                None
            }
        },
        // The rclone family (#63): serve spawn + readiness is async, setup
        // is sync, both under the app runtime (same shape as boot). The
        // webdav lane (#83) rides the same family — its constructor reads
        // the vault-filled IUM_WEBDAV_* envs and obscures the password.
        // The google_drive lane (#84) rides it too — its token was written
        // by the browser sign-in (rclone's own OAuth client); no obscure
        // dance, the token is already rclone's own JSON.
        "rclone_bridge" | "mega" | "webdav" | "google_drive" => {
            let built = tauri::async_runtime::block_on(async {
                let cfg = if id == "mega" {
                    ium_core::adapter_rclone::RcloneBridgeConfig::mega_from_env().await
                } else if id == "webdav" {
                    ium_core::adapter_rclone::RcloneBridgeConfig::webdav_from_env().await
                } else if id == "google_drive" {
                    ium_core::adapter_rclone::RcloneBridgeConfig::google_drive_from_env()
                } else {
                    ium_core::adapter_rclone::RcloneBridgeConfig::from_env()
                };
                match cfg {
                    Ok(cfg) => ium_core::adapter_rclone::RcloneBridgeAdapter::start(cfg)
                        .await
                        .map(Some)
                        .map_err(|e| e.to_string()),
                    Err(_) => Ok(None), // no env credentials — no lane, honestly
                }
            });
            match built {
                Ok(Some(adapter)) => Some(Arc::new(adapter)),
                Ok(None) => {
                    tracing::warn!("{id} not configured (no env credentials)");
                    None
                }
                Err(e) => {
                    tracing::warn!("{id} lane unavailable: {e}");
                    None
                }
            }
        }
        _ => None,
    }
}

/// Register the protocol adapters this build ships with. The always-on
/// lanes first, then the credential-gated families through the shared
/// factory. A machine without credentials for a lane simply has no such
/// entry in the registry — the UI reflects the registry, it never assumes.
fn register_adapters(manager: &mut JobManager) {
    manager.register_adapter(Arc::new(HttpOneShotAdapter::new().expect("http client")));
    manager.register_adapter(Arc::new(TusAdapter::new()));
    for id in [
        "s3_multipart",
        "terabox",
        "feishu",
        "rclone_bridge",
        "mega",
        "webdav",
        "google_drive",
        "youtube",
    ] {
        if let Some(adapter) = build_adapter(id) {
            manager.register_adapter(adapter);
        } // the factory already warned; absence is the honest state
    }
}

/// The settings UI renders forms from the schema; secret VALUES never
/// leave the process (`value: null, present: true`) (#69).
#[derive(serde::Serialize)]
struct FieldView {
    label: &'static str,
    secret: bool,
    required: bool,
    present: bool,
    value: Option<String>,
    /// No input renders (#84): the OAuth lanes' token row is a saved-state
    /// chip — the sign-in writes it inside the process, the UI never sees
    /// a token byte, its PRESENCE is the state.
    hidden: bool,
}

#[tauri::command]
fn credentials_schema() -> Vec<vault::AdapterSchema> {
    vault::schema()
}

#[tauri::command]
fn credentials_get(
    app: tauri::AppHandle,
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, FieldView>> {
    let data = vault(&app).load();
    let mut out = std::collections::BTreeMap::new();
    for a in vault::schema() {
        let stored = data.adapters.get(a.adapter);
        let mut fields = std::collections::BTreeMap::new();
        for f in a.fields {
            let stored_val = stored.and_then(|e| e.get(f.name));
            let (value, present) = if f.secret {
                (None, stored_val.is_some())
            } else {
                (stored_val.cloned(), stored_val.is_some())
            };
            fields.insert(
                f.name.to_string(),
                FieldView {
                    label: f.label,
                    secret: f.secret,
                    required: f.required,
                    present,
                    value,
                    hidden: f.hidden,
                },
            );
        }
        out.insert(a.adapter.to_string(), fields);
    }
    out
}

/// PARTIAL merge with a schema guard: absent field = keep (so the UI can
/// leave masked secrets untouched), empty = delete; unknown adapters and
/// fields are refused — the vault is not a generic KV store.
#[tauri::command]
fn credentials_set(
    app: tauri::AppHandle,
    adapter: String,
    fields: std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let schema = vault::schema();
    let Some(a) = schema.iter().find(|s| s.adapter == adapter) else {
        return Err(format!("unknown adapter: {adapter}"));
    };
    for k in fields.keys() {
        if !a.fields.iter().any(|f| f.name == *k) {
            return Err(format!("unknown field for {adapter}: {k}"));
        }
    }
    vault(&app).set(&adapter, fields)
}

#[tauri::command]
fn credentials_delete(app: tauri::AppHandle, adapter: String) -> Result<(), String> {
    vault(&app).delete(&adapter)
}

/// What a hot apply did (#81): `replaced` (a known lane swapped config in
/// place), `added` (a first credential entry registered a lane boot never
/// had), or `removed` (credentials gone → the lane is gone). `lanes` is
/// the registry AFTER the apply — the UI's adapter select reflects it.
#[derive(serde::Serialize)]
struct ApplyOutcome {
    action: &'static str,
    lanes: Vec<String>,
}

/// The hot apply's shared body (#81): rebuild the lane from vault+env
/// through the shared factory and upsert or remove it — the NEXT job uses
/// the fresh config; in-flight jobs keep the Arc they resolved at
/// create_job (the engine's non-retroactive contract). No restart.
fn hot_apply(engine: &Engine, adapter: &str) -> Result<ApplyOutcome, String> {
    let action = match build_adapter(adapter) {
        Some(a) => {
            if engine.0.manager().replace_adapter(a) {
                "replaced"
            } else {
                "added"
            }
        }
        None => {
            engine.0.manager().remove_adapter(adapter);
            "removed"
        }
    };
    Ok(ApplyOutcome {
        action,
        lanes: engine
            .0
            .manager()
            .adapter_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
    })
}

/// The hot apply (#81): called by the settings UI right after a save or a
/// removal. Removing credentials removes the lane.
#[tauri::command]
fn apply_credentials(
    engine: State<Engine>,
    app: tauri::AppHandle,
    adapter: String,
) -> Result<ApplyOutcome, String> {
    vault(&app).apply_env();
    hot_apply(&engine, &adapter)
}

/// The browser sign-in (#82): `rclone authorize <backend>` opens the
/// provider's real sign-in page in the user's browser; they sign in as
/// normal (2FA and consent screens included — the provider owns the
/// flow); rclone prints the token config; the shell parses it, merges the
/// env lines into the bridge lane's `extra_env` INSIDE the process (a
/// masked secret value never crosses the UI, #69), and hot-applies the
/// lane. The token rides a secret field, so the OS keychain stores it.
/// The allowlist below is the injection contract; Google types are absent
/// by owner ruling (#59/#60 shelved).
#[tauri::command]
async fn authorize_oauth(
    engine: State<'_, Engine>,
    app: tauri::AppHandle,
    backend: String,
    bin: Option<String>,
) -> Result<ApplyOutcome, String> {
    if !auth::is_oauth_backend(&backend) {
        return Err(format!("not a browser-sign-in backend: {backend}"));
    }
    let bin = bin
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "rclone".to_string());
    // The spawn+wait blocks for as long as the user takes in their browser
    // — it runs on the blocking pool, never on a runtime worker.
    let backend_for_spawn = backend.clone();
    let run = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new(&bin)
            .arg("authorize")
            .arg(&backend_for_spawn)
            .output()
            .map_err(|e| {
                format!(
                    "could not run {bin} — install rclone or point the bridge lane's bin field at it. ({e})"
                )
            })
    })
    .await
    .map_err(|e| format!("the browser sign-in task failed: {e}"))?;
    let out = run?;
    if !out.status.success() {
        return Err(format!(
            "the browser sign-in did not finish ({}). {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let token = auth::parse_authorize_output(&String::from_utf8_lossy(&out.stdout))?;
    let lines: Vec<String> = auth::env_lines(&backend, &token)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let stored = vault(&app)
        .load()
        .adapters
        .get("rclone_bridge")
        .and_then(|m| m.get("extra_env"))
        .cloned()
        .unwrap_or_default();
    let merged = auth::merge_env_lines(&stored, &lines);
    vault(&app).set(
        "rclone_bridge",
        BTreeMap::from([("extra_env".to_string(), merged)]),
    )?;
    // The first-class lanes (#84): the sign-in also writes the token under
    // the lane's own secret — the hidden token row's presence IS the
    // lane's connection state, and apply_env hands it to the core
    // constructor (IUM_GOOGLE_DRIVE_TOKEN). No token byte crosses the UI.
    if backend == "drive" {
        vault(&app).set(
            "google_drive",
            BTreeMap::from([("token".to_string(), token)]),
        )?;
    }
    vault(&app).apply_env();
    let applied = hot_apply(&engine, "rclone_bridge")?;
    if backend == "drive" {
        hot_apply(&engine, "google_drive")?;
    }
    Ok(applied)
}

/// Open the user's default browser at `url` — the provider owns the flow
/// (the OS opener, per-OS one-liner; no extra shell dependency).
fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let spawn = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let spawn = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawn = std::process::Command::new("xdg-open").arg(url).spawn();
    spawn
        .map(|_| ())
        .map_err(|e| format!("could not open the browser at {url} ({e})"))
}

/// The loopback redirect server (#85): ONE `GET /?code=…&state=…`, the
/// anti-CSRF state verified, one honest landing sentence, and the code is
/// the answer. Ten minutes of patience — the user takes as long as they
/// take (2FA included); then the honest timeout. Blocks: it runs on the
/// blocking pool, never on a runtime worker.
fn wait_for_code(expect_state: String) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    let listener =
        TcpListener::bind(("127.0.0.1", ium_core::google::REDIRECT_PORT)).map_err(|e| {
            format!(
                "could not listen on localhost:{} — another IUM sign-in may be in flight. ({e})",
                ium_core::google::REDIRECT_PORT
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    let ok_page = "<html><body style=\"font-family:sans-serif;background:#f8fafc;color:#0f172a\"><p>Sign-in complete. You can close this window and return to IUM.</p></body></html>";
    let bad_page = "<html><body style=\"font-family:sans-serif;background:#f8fafc;color:#0f172a\"><p>IUM did not receive a valid sign-in. Close this window and try again.</p></body></html>";
    while Instant::now() < deadline {
        let (mut stream, _) = listener
            .accept()
            .map_err(|e| format!("the loopback listener failed: {e}"))?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match stream.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16_384 {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let request_line = head.lines().next().unwrap_or("");
        let target = request_line.split_whitespace().nth(1).unwrap_or("/");
        let answered = match ium_core::google::parse_redirect(target) {
            Ok((code, state)) if ium_core::google::state_matches(&state, &expect_state) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    ok_page.len(),
                    ok_page
                );
                let _ = stream.flush();
                Some(code)
            }
            _ => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    bad_page.len(),
                    bad_page
                );
                let _ = stream.flush();
                None
            }
        };
        if let Some(code) = answered {
            return Ok(code);
        }
        // A favicon probe or a stray request: keep listening.
    }
    Err("the browser sign-in timed out after ten minutes — nothing was connected.".into())
}

/// The YouTube sign-in (#85): the loopback OAuth flow. The client id/secret
/// ride the DRAFTS (the sheet's two fields) with the saved values as
/// fallback — a one-time setup, after which connect is one click. The
/// token lands under the lane's own hidden secret (OS keychain, #81) and
/// the lane hot-applies — the drawer closes itself (the reflex).
#[tauri::command]
async fn connect_youtube(
    engine: State<'_, Engine>,
    app: tauri::AppHandle,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<ApplyOutcome, String> {
    let saved = |k: &str| {
        vault(&app)
            .load()
            .adapters
            .get("youtube")
            .and_then(|m| m.get(k))
            .cloned()
    };
    let draft = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let id = draft(client_id).or_else(|| saved("client_id"));
    let secret = draft(client_secret).or_else(|| saved("client_secret"));
    let (Some(id), Some(secret)) = (id, secret) else {
        return Err("The YouTube client id and secret are required — paste them from your Google Cloud project once (console.cloud.google.com → OAuth client, type Desktop app, redirect URI http://localhost:8737).".into());
    };
    let client = ium_core::GoogleClient {
        client_id: id,
        client_secret: secret,
    };
    let (verifier, _challenge) = ium_core::google::pkce_pair();
    let state = ium_core::google::new_state();
    let url = ium_core::google::authorize_url(&client.client_id, &verifier, &state);
    open_in_browser(&url)?;
    let expect_state = state;
    let code = tauri::async_runtime::spawn_blocking(move || wait_for_code(expect_state))
        .await
        .map_err(|e| format!("the sign-in task failed: {e}"))??;
    let fresh = client.exchange_code(&code, &verifier).await?;
    vault(&app).set(
        "youtube",
        BTreeMap::from([("token".to_string(), fresh.to_json())]),
    )?;
    vault(&app).apply_env();
    hot_apply(&engine, "youtube")
}

/// What Test-connection learned (#81). `checked = false` is the honest
/// "verified on the first upload" answer — never a fake green check.
/// `final = true` means `sentence` is already human copy (a missing field);
/// `false` means it is the RAW engine string, for the UI's copy table.
#[derive(serde::Serialize)]
struct ProbeOutcome {
    checked: bool,
    ok: bool,
    sentence: String,
    /// `final` is a reserved Rust keyword — the wire name stays `final`
    /// (the TS contract) via the rename.
    #[serde(rename = "final")]
    final_copy: bool,
}

fn probe_ok() -> ProbeOutcome {
    ProbeOutcome {
        checked: true,
        ok: true,
        sentence: "The server accepted the credentials.".into(),
        final_copy: true,
    }
}

fn probe_missing(sentence: &str) -> ProbeOutcome {
    ProbeOutcome {
        checked: true,
        ok: false,
        sentence: sentence.into(),
        final_copy: true,
    }
}

fn probe_raw(r: Result<(), ium_core::adapter::AdapterError>) -> ProbeOutcome {
    match r {
        Ok(()) => probe_ok(),
        Err(e) => ProbeOutcome {
            checked: true,
            ok: false,
            sentence: e.to_string(),
            final_copy: false,
        },
    }
}

/// The Test-connection check (#81): builds the family's config from the
/// DRAFTS in memory — never persisted, never env — and runs that family's
/// read-only credential check (metadata reads only; NO upload session is
/// created, so a Test button can never orphan a real upload). Families
/// whose endpoint is per-job (the s3 family) answer honestly: not checked.
#[tauri::command]
async fn probe_credentials(
    app: tauri::AppHandle,
    adapter: String,
    fields: BTreeMap<String, String>,
) -> ProbeOutcome {
    let draft = |k: &str| {
        fields
            .get(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    match adapter.as_str() {
        "terabox" => {
            let Some(ndus) = draft("ndus") else {
                return probe_missing("The ndus cookie is required for a test — paste it first.");
            };
            let cfg = TeraboxConfig {
                ndus,
                app_id: draft("app_id").unwrap_or_else(|| "250528".into()),
                base: draft("base").unwrap_or_else(|| "https://dm.terabox.com".into()),
            };
            probe_raw(TeraboxAdapter::new(cfg).check_credentials().await)
        }
        "feishu" => {
            let (Some(app_id), Some(app_secret)) = (draft("app_id"), draft("app_secret")) else {
                return probe_missing(
                    "Both the app id and the app secret are required for a test.",
                );
            };
            let a = ium_core::adapter_feishu::FeishuAdapter::new(
                ium_core::adapter_feishu::FeishuConfig::new(app_id, app_secret),
            );
            probe_raw(a.check_credentials().await)
        }
        "mega" => {
            let (Some(user), Some(pass)) = (draft("user"), draft("pass")) else {
                return probe_missing("Your MEGA email and password are required for a test.");
            };
            let obscured = ium_core::adapter_rclone::RcloneBridgeConfig::rclone_obscure(
                &std::path::PathBuf::from("rclone"),
                &pass,
            )
            .await;
            let Ok(obscured) = obscured else {
                return probe_missing(
                    "The rclone binary is required for a test — install it or point the bridge lane's bin field at it.",
                );
            };
            let Ok(mut cfg) = ium_core::adapter_rclone::RcloneBridgeConfig::new(
                ium_core::adapter_rclone::RcloneBridgeMode::Spawn,
                "mega:",
            ) else {
                return probe_missing("The MEGA remote name is required for a test.");
            };
            cfg.bucket = draft("bucket").unwrap_or_else(|| "ium".into());
            cfg.lane_name = ium_core::adapter_rclone::MEGA_LANE;
            cfg.extra_env = vec![
                ("RCLONE_CONFIG_MEGA_TYPE".into(), "mega".into()),
                ("RCLONE_CONFIG_MEGA_USER".into(), user),
                ("RCLONE_CONFIG_MEGA_PASS".into(), obscured),
            ];
            if let Some(code) = draft("twofa") {
                cfg.extra_env.push(("RCLONE_CONFIG_MEGA_2FA".into(), code));
            }
            bridge_probe(cfg).await
        }
        "webdav" => {
            let (Some(url), Some(username), Some(password)) =
                (draft("url"), draft("username"), draft("password"))
            else {
                return probe_missing(
                    "The server URL, your username and the password are required for a test.",
                );
            };
            let bin = draft("bin").unwrap_or_else(|| "rclone".into());
            let obscured = ium_core::adapter_rclone::RcloneBridgeConfig::rclone_obscure(
                &std::path::PathBuf::from(&bin),
                &password,
            )
            .await;
            let Ok(obscured) = obscured else {
                return probe_missing(
                    "The rclone binary is required for a test — install it or point the bridge lane's bin field at it.",
                );
            };
            let Ok(cfg) = ium_core::adapter_rclone::RcloneBridgeConfig::webdav_config_from_parts(
                &url,
                &username,
                &obscured,
                draft("vendor").unwrap_or_default().as_str(),
                &draft("bucket").unwrap_or_else(|| "ium".into()),
                &bin,
            ) else {
                return probe_missing("The WebDAV configuration was refused.");
            };
            bridge_probe(cfg).await
        }
        "youtube" => {
            let (Some(id), Some(secret)) = (draft("client_id"), draft("client_secret")) else {
                return probe_missing(
                    "The client id and the client secret are required for a test.",
                );
            };
            // The token is a hidden row: no draft exists, the test reads
            // the SAVED sign-in.
            let token = vault(&app)
                .load()
                .adapters
                .get("youtube")
                .and_then(|m| m.get("token"))
                .cloned();
            let Some(token) = token else {
                return probe_missing(
                    "No Google sign-in yet — connect YouTube once and the token lives in your keychain.",
                );
            };
            match ium_core::YoutubeConfig::from_parts(&id, &secret, &token, "", "") {
                Ok(cfg) => probe_raw(ium_core::YoutubeAdapter::new(cfg).check_credentials().await),
                Err(e) => probe_missing(&e.to_string()),
            }
        }
        "google_drive" => {
            // The token is a hidden row: no draft exists, the test reads
            // the SAVED sign-in (vault first, the generic bridge's merged
            // env lines second — one credential, two honest seats).
            let data = vault(&app).load();
            let token = data
                .adapters
                .get("google_drive")
                .and_then(|m| m.get("token"))
                .cloned()
                .or_else(|| {
                    data.adapters
                        .get("rclone_bridge")
                        .and_then(|m| m.get("extra_env"))
                        .and_then(|extra| {
                            extra
                                .lines()
                                .find(|l| l.starts_with("RCLONE_CONFIG_IUM_DRIVE_TOKEN="))
                                .map(|l| l.split_once('=').unwrap().1.to_string())
                        })
                });
            let Some(token) = token else {
                return probe_missing(
                    "No Google sign-in yet — connect Google Drive once and the token lives in your keychain.",
                );
            };
            let bin = fields
                .get("bin")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    data.adapters
                        .get("rclone_bridge")
                        .and_then(|m| m.get("bin"))
                        .cloned()
                })
                .unwrap_or_else(|| "rclone".into());
            let bucket = data
                .adapters
                .get("google_drive")
                .and_then(|m| m.get("bucket"))
                .cloned()
                .unwrap_or_else(|| "ium".into());
            match ium_core::adapter_rclone::RcloneBridgeConfig::google_drive_config_from_parts(
                &token, &bucket, &bin,
            ) {
                Ok(cfg) => bridge_probe(cfg).await,
                Err(e) => probe_missing(&e.to_string()),
            }
        }
        "rclone_bridge" => {
            let Some(remote) = draft("remote") else {
                return probe_missing("The remote name is required for a test.");
            };
            let Ok(mut cfg) = ium_core::adapter_rclone::RcloneBridgeConfig::new(
                ium_core::adapter_rclone::RcloneBridgeMode::Spawn,
                remote,
            ) else {
                return probe_missing("The remote name is required for a test.");
            };
            cfg.rclone_bin =
                std::path::PathBuf::from(draft("bin").unwrap_or_else(|| "rclone".into()));
            cfg.bucket = draft("bucket").unwrap_or_else(|| "ium".into());
            if let Some(extra) = draft("extra_env") {
                for line in extra.lines() {
                    let line = line.trim();
                    if let Some((k, v)) = line.split_once('=') {
                        // The vault's own rule: only rclone's env-config
                        // namespace travels (foreign keys are refused).
                        if k.starts_with("RCLONE_CONFIG_") {
                            cfg.extra_env.push((k.to_string(), v.to_string()));
                        }
                    }
                }
            }
            bridge_probe(cfg).await
        }
        // The s3 family (and any family without a no-job checkable
        // endpoint): the honest not-checked answer.
        _ => ProbeOutcome {
            checked: false,
            ok: true,
            sentence: "Not checked here — this lane verifies itself on the first upload.".into(),
            final_copy: true,
        },
    }
}

/// The bridge families' test: the serve must spawn, reach readiness and
/// expose the bucket — that proves the binary, the config lines and the
/// remote wiring. The remote's own credentials answer on the first upload;
/// the sentence says exactly that (never a fake green check).
async fn bridge_probe(cfg: ium_core::adapter_rclone::RcloneBridgeConfig) -> ProbeOutcome {
    match ium_core::adapter_rclone::RcloneBridgeAdapter::start(cfg).await {
        Ok(_) => ProbeOutcome {
            checked: true,
            ok: true,
            sentence: "The bridge is up — the remote answers on the first upload.".into(),
            final_copy: true,
        },
        Err(e) => ProbeOutcome {
            checked: true,
            ok: false,
            sentence: e.to_string(),
            final_copy: false,
        },
    }
}

/// The footer's honesty marker (#81): which store actually holds the
/// secret values on this machine.
#[tauri::command]
fn secret_mode(app: tauri::AppHandle) -> String {
    vault(&app).mode().to_string()
}

#[tauri::command]
fn engine_status(engine: State<Engine>) -> String {
    format!(
        "ium-core v{} ready — adapters: {}",
        ium_core::VERSION,
        engine.0.manager().adapter_names().join(", ")
    )
}

#[tauri::command]
fn create_job(
    engine: State<Engine>,
    app: tauri::AppHandle,
    spec: JobSpec,
    max_conns: u32,
) -> Result<JobRecord, String> {
    let record = engine
        .0
        .manager()
        .create_job(&spec, max_conns)
        .map_err(engine_error)?;
    engine.0.drive(&record.id);
    spawn_watcher(app, engine.0.clone(), record.id.clone());
    Ok(record)
}

#[tauri::command]
fn list_jobs(engine: State<Engine>) -> Result<Vec<JobRecord>, String> {
    engine.0.manager().list_jobs().map_err(engine_error)
}

#[tauri::command]
fn snapshot(engine: State<Engine>, job_id: String) -> Result<JobSnapshot, String> {
    engine.0.manager().snapshot(&job_id).map_err(engine_error)
}

/// Spec Sprint-1 item 9 name for the per-chunk inspector drawer (#16).
#[tauri::command]
fn inspect_chunks(engine: State<Engine>, job_id: String) -> Result<JobSnapshot, String> {
    engine.0.manager().snapshot(&job_id).map_err(engine_error)
}

#[tauri::command]
fn pause_job(engine: State<Engine>, job_id: String) -> Result<(), String> {
    engine.0.pause(&job_id).map_err(engine_error)
}

#[tauri::command]
fn resume_signal(
    engine: State<Engine>,
    app: tauri::AppHandle,
    job_id: String,
) -> Result<(), String> {
    // Re-drives the job (the old task exited when it journaled Paused) and
    // restarts the event watcher for it.
    engine.0.resume_signal(&job_id).map_err(engine_error)?;
    spawn_watcher(app, engine.0.clone(), job_id);
    Ok(())
}

/// Cancel: journaled `Canceled` FIRST, then the drive task is aborted —
/// a user cancel is never recorded as `Failed` (honest endings, issue #15).
#[tauri::command]
async fn cancel_job(engine: State<'_, Engine>, job_id: String) -> Result<JobRecord, String> {
    engine.0.cancel(&job_id).await.map_err(engine_error)
}

/// The create-job dialog needs the size the engine's JobSpec requires; the
/// shell stats, the engine plans (the UI never computes transfer truth).
#[tauri::command]
fn stat_file(path: String) -> Result<u64, String> {
    std::fs::metadata(&path)
        .map(|m| m.len())
        .map_err(|e| format!("{}: {e}", path))
}

/// `job_updated` stream: journal snapshots on a fixed interval until the
/// job is terminal (the terminal snapshot is emitted, then the watcher
/// stops — a finished job generates no events forever).
fn spawn_watcher(app: tauri::AppHandle, sup: Arc<Supervisor>, job_id: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let Ok(snap) = sup.manager().snapshot(&job_id) else {
                return;
            };
            let terminal = snap.job.state.is_terminal();
            let _ = app.emit("job_updated", &snap);
            if terminal {
                // Lane 7 (#75): the usage event rides the FINAL snapshot's
                // non-user fields only — adapter, terminal state, wide
                // buckets. `source_path` and `endpoint` sit on the same
                // record and are structurally unable to reach an event.
                if let Some(t) = TELEMETRY.get() {
                    let duration = snap.job.updated_at - snap.job.created_at;
                    t.record_job_terminal(
                        &snap.job.adapter,
                        snap.job.state.as_str(),
                        snap.job.source_size,
                        duration,
                    );
                }
                return;
            }
        }
    });
}

fn payload_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let p = info.payload();
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// The telemetry Privacy commands (#75): consent + buffer, schema-flat.
#[derive(serde::Serialize)]
struct TelemetryView {
    consent: bool,
    count: usize,
    path: String,
}

#[tauri::command]
fn telemetry_status() -> TelemetryView {
    match TELEMETRY.get() {
        Some(t) => {
            let (consent, count) = t.status();
            TelemetryView {
                consent,
                count,
                path: t.buf_path().display().to_string(),
            }
        }
        None => TelemetryView {
            consent: true,
            count: 0,
            path: String::new(),
        },
    }
}

#[tauri::command]
fn telemetry_set_consent(on: bool) -> Result<(), String> {
    match TELEMETRY.get() {
        Some(t) => t.set_consent(on),
        None => Err("telemetry is not ready yet".to_string()),
    }
}

#[tauri::command]
fn telemetry_clear() -> Result<(), String> {
    match TELEMETRY.get() {
        Some(t) => t.clear(),
        None => Err("telemetry is not ready yet".to_string()),
    }
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Lane 7 (#75): the telemetry pipeline installs FIRST — the
            // panic hook must be in place before any risky work, and the
            // buffer lives in the app-data dir (the journal's directory;
            // the vault keeps the config dir). The hook CHAINS to the
            // default: the real panic still prints, the event rides the
            // charter's allowlist (verbatim message + IUM source location).
            let tel_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            let _ = std::fs::create_dir_all(&tel_dir);
            let tel = Arc::new(telemetry::Telemetry::open(&tel_dir));
            let _ = TELEMETRY.set(tel.clone());
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if let Some(t) = TELEMETRY.get() {
                    let loc = info.location();
                    t.record_panic(
                        &payload_message(info),
                        loc.map(|l| l.file()).unwrap_or("unknown"),
                        loc.map(|l| l.line()).unwrap_or(0),
                        loc.map(|l| l.column()).unwrap_or(0),
                    );
                }
                default_hook(info);
            }));
            // The vault fills the env gaps BEFORE adapters construct (env
            // wins: real env vars are never overwritten) — the *_from_env
            // constructors behave identically, vault-configured machines
            // just get the same registry env-configured ones do (#69).
            let injected = vault::Vault::open(&vault_dir(app.handle())).apply_env();
            if injected > 0 {
                tracing::info!("vault: injected {injected} credential variables");
            }
            let manager = JobManager::open(db_path(app.handle())).map_err(|e| e.to_string())?;
            let mut manager = manager;
            register_adapters(&mut manager);
            // Issue #39: setup runs on the main thread — the same thread
            // tauri runs SYNC commands on — and no tokio context is entered
            // there. The supervisor must spawn drive tasks through an
            // explicit handle (tauri's async runtime, whose tokio handle we
            // read from inside a block_on-entered context); a raw
            // tokio::spawn in create_job/resume_signal panicked here and
            // aborted the whole shell on the first Upload click
            // ("Session terminated without a reply", CI #65–#70).
            let runtime_handle =
                tauri::async_runtime::block_on(async { tokio::runtime::Handle::current() });
            // Product bar #66 lane 5: the intake can queue faster than any
            // uplink drains — 4 simultaneous drives (aria2 ships 5 for
            // downloads; uploads are heavier), the rest FIFO-queued in the
            // supervisor. Queued jobs read Created in the journal, so the
            // UI needs no new state.
            app.manage(Engine(Arc::new(Supervisor::with_handle_and_limit(
                Arc::new(manager),
                runtime_handle,
                4,
            ))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            list_adapters,
            create_job,
            list_jobs,
            snapshot,
            inspect_chunks,
            pause_job,
            resume_signal,
            cancel_job,
            stat_file,
            credentials_schema,
            credentials_get,
            credentials_set,
            credentials_delete,
            apply_credentials,
            authorize_oauth,
            connect_youtube,
            probe_credentials,
            secret_mode,
            telemetry_status,
            telemetry_set_consent,
            telemetry_clear,
        ])
        .plugin(tauri_plugin_dialog::init())
        // the auto-update lane (#80): check = automatic, install = user-armed;
        // endpoint + pubkey live in tauri.conf.json (the #71 rotated key)
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .run(tauri::generate_context!())
        .expect("error while running IUM");
}
