//! The credentials vault (#66 Lane 2, #69): product-grade storage for
//! adapter credentials — the `.secrets/` contract turned into a surface a
//! normal user can drive. Self-contained pure-std (no tauri imports): the
//! sandbox has no WebKitGTK (the #61 lesson), so this module's logic is
//! testable LOCALLY via `rustc --test vault.rs`, while the crate that
//! embeds it takes its compile proof from CI.
//!
//! Storage: `credentials.json` in the app-config dir, adapter → {field →
//! value}, written 0600 (best-effort on Windows), atomic write (tmp +
//! rename). PRECEDENCE (env wins): `apply_env` never overwrites a variable
//! the process already has — CI/dev machines keep today's behavior; the
//! vault fills the gaps. The single source of truth for the mapping is the
//! core constructors' env contract (`*_from_env`).
//!
//! SECRET VALUES (#81): the spec pins the OS keychain (docs/02 §Secrets —
//! Credential Manager / Keychain / Secret Service). The [`SecretStore`]
//! seam carries secret values; the real impl (the `keyring` crate) lives
//! in lib.rs so this module stays pure-std, and tests inject an in-memory
//! double to pin the contract deterministically. When the OS has no secret
//! service (headless Linux, CI), the vault falls back to the 0600 file and
//! the JSON carries the honest `secret_mode` marker the UI prints.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const VAULT_FILE: &str = "credentials.json";

/// One credential field as the settings UI renders it.
#[derive(serde::Serialize)]
pub struct FieldSchema {
    pub name: &'static str,
    pub label: &'static str,
    pub secret: bool,
    pub required: bool,
    /// No input renders (#84): the OAuth lanes' token row is a saved-state
    /// chip — the browser sign-in writes it INSIDE the process, the UI only
    /// hears the outcome. Presence still counts for the connection state.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(serde::Serialize)]
pub struct AdapterSchema {
    pub adapter: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub fields: Vec<FieldSchema>,
}

/// The schema — every field maps to an env name a core constructor reads;
/// the table in #69 is generated FROM this, never by hand twice.
pub fn schema() -> Vec<AdapterSchema> {
    fn f(name: &'static str, label: &'static str, secret: bool, required: bool) -> FieldSchema {
        FieldSchema {
            name,
            label,
            secret,
            required,
            hidden: false,
        }
    }
    /// The hidden required secret (#84): written by the browser sign-in
    /// inside the process, never rendered as an input, never read back by
    /// the UI — its PRESENCE is the lane's connection state.
    fn h(name: &'static str, label: &'static str) -> FieldSchema {
        FieldSchema {
            name,
            label,
            secret: true,
            required: true,
            hidden: true,
        }
    }
    vec![
        AdapterSchema {
            adapter: "s3_multipart",
            label: "S3 multipart",
            note: "Any S3-compatible endpoint (AWS R2 MinIO SeaweedFS…).",
            fields: vec![
                f("access_key", "Access key", true, true),
                f("secret_key", "Secret key", true, true),
                f("region", "Region", false, true),
            ],
        },
        AdapterSchema {
            adapter: "terabox",
            label: "TeraBox",
            note: "The ndus cookie from a logged-in browser session.",
            fields: vec![
                f("ndus", "ndus cookie", true, true),
                f("app_id", "App id", false, false),
                f("base", "API base URL", false, false),
            ],
        },
        AdapterSchema {
            adapter: "mega",
            label: "MEGA (rclone bridge)",
            note: "Your MEGA login. Optional 2FA code if the account has it.",
            fields: vec![
                f("user", "Email", false, true),
                f("pass", "Password", true, true),
                f("twofa", "2FA code", true, false),
                f("bucket", "Target folder", false, false),
            ],
        },
        AdapterSchema {
            adapter: "rclone_bridge",
            label: "rclone bridge (advanced)",
            note: "Any rclone remote. Extra env lines: one KEY=VALUE per line — keys must start with RCLONE_CONFIG_ (rclone's own env-config practice).",
            fields: vec![
                f("remote", "Remote name", false, true),
                f("mode", "Mode (spawn|connect)", false, false),
                f("bucket", "Target folder", false, false),
                f("bin", "rclone binary path", false, false),
                f("extra_env", "Extra rclone env (KEY=VALUE per line)", true, false),
            ],
        },
        AdapterSchema {
            adapter: "feishu",
            label: "Feishu Drive",
            note: "Tenant app credentials from the Feishu open platform.",
            fields: vec![
                f("app_id", "App id", false, true),
                f("app_secret", "App secret", true, true),
            ],
        },
        AdapterSchema {
            adapter: "webdav",
            label: "WebDAV",
            note: "Nextcloud, ownCloud or any WebDAV server: its URL, your login, an optional app password.",
            fields: vec![
                f("url", "Server URL", false, true),
                f("username", "Username", false, true),
                f("password", "Password / app password", true, true),
                f("vendor", "Vendor (nextcloud, owncloud, sharepoint — optional)", false, false),
                f("bucket", "Target folder", false, false),
            ],
        },
        AdapterSchema {
            adapter: "google_drive",
            label: "Google Drive",
            note: "Sign in with your Google account in the browser — no fields to fill. The sign-in token lives in your OS keychain.",
            fields: vec![h("token", "Google sign-in")],
        },
        AdapterSchema {
            adapter: "youtube",
            label: "YouTube",
            note: "Upload videos by connecting your Google account. YouTube's upload scope needs a registered client: paste your Google Cloud project's client id and secret once — after that, connect is one click.",
            fields: vec![
                f("client_id", "Client id (…apps.googleusercontent.com)", false, true),
                f("client_secret", "Client secret", true, true),
                h("token", "Google sign-in"),
            ],
        },
    ]
}

/// Where secret VALUES live (spec §Secrets): "keyring" = the OS keychain
/// via the [`SecretStore`] seam; "file" = the honest 0600-file fallback
/// (the OS secret service is unavailable — headless Linux, CI). The marker
/// rides the vault JSON so the UI can say which one is real.
pub const MODE_KEYRING: &str = "keyring";
pub const MODE_FILE: &str = "file";

/// The keychain seam (#81): only SECRET field values ever cross it — one
/// entry per `{adapter}.{field}`. The real impl is the OS keychain (the
/// `keyring` crate, wired in lib.rs so this module stays pure-std); tests
/// inject an in-memory double. `get` on a missing entry is `Ok(None)`, not
/// an error — absence is a fact, not a failure.
pub trait SecretStore: Send + Sync {
    /// Store (or overwrite) the secret for one `{adapter}.{field}` account.
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    /// The secret, or `None` when the keychain has no entry for it.
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    /// Remove the entry; deleting an absent entry is an honest `Ok`.
    fn delete(&self, account: &str) -> Result<(), String>;
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug)]
pub struct VaultData {
    pub adapters: BTreeMap<String, BTreeMap<String, String>>,
    /// The secret-store mode this file was last written under — the UI's
    /// honesty marker ("Secrets live in your OS keychain" vs the fallback
    /// sentence). `None` = a pre-#81 file (plaintext era, reads still work).
    #[serde(default)]
    pub secret_mode: Option<String>,
}

pub struct Vault {
    path: PathBuf,
    /// The keychain seam. `None` = the honest file fallback (pure-std
    /// default — the `rustc --test vault.rs` path); `Some` = secret values
    /// ride the store and NEVER touch the file.
    store: Option<Box<dyn SecretStore>>,
}

impl Vault {
    /// The file fallback (pure-std default). The product entry point is
    /// [`Vault::open_keychain`] — lib.rs probes the OS secret service and
    /// passes the real store or `None`.
    pub fn open(dir: &Path) -> Self {
        Self {
            path: dir.join(VAULT_FILE),
            store: None,
        }
    }

    /// The product constructor (#81): `Some(store)` = keychain mode (secret
    /// values never land in the file); `None` = the 0600-file fallback with
    /// the honest marker (the OS secret service is unavailable).
    pub fn open_keychain(dir: &Path, store: Option<Box<dyn SecretStore>>) -> Self {
        Self {
            path: dir.join(VAULT_FILE),
            store,
        }
    }

    /// `"keyring"` when secret values ride the keychain seam, `"file"` for
    /// the fallback — the UI's honesty sentence is built from this.
    pub fn mode(&self) -> &'static str {
        if self.store.is_some() {
            MODE_KEYRING
        } else {
            MODE_FILE
        }
    }

    /// A missing file is an empty vault, not an error; a corrupted file is
    /// an empty vault too — credentials are re-enterable, panic is not an
    /// option in a startup path. In keychain mode the secret fields are
    /// overlaid FROM the store (never the file); a legacy plaintext copy in
    /// the file still answers while the keychain has no entry — the lazy
    /// migration: old values keep working until the first save, then the
    /// file is scrubbed.
    pub fn load(&self) -> VaultData {
        let mut data: VaultData = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if let Some(store) = &self.store {
            for a in schema() {
                for f in a.fields {
                    if !f.secret {
                        continue;
                    }
                    let account = format!("{}.{}", a.adapter, f.name);
                    if let Ok(Some(v)) = store.get(&account) {
                        data.adapters
                            .entry(a.adapter.to_string())
                            .or_default()
                            .insert(f.name.to_string(), v);
                    }
                }
            }
        }
        data
    }

    /// Atomic write: tmp file in the same dir + rename; 0600 best-effort.
    pub fn save(&self, data: &VaultData) -> Result<(), String> {
        let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    /// Partial merge: absent field = keep, empty value = delete. Secret
    /// fields go to the keychain seam FIRST (a failed keychain write fails
    /// loud BEFORE the file is touched — nothing is lost); in keychain mode
    /// any legacy plaintext secret for this adapter is scrubbed from the
    /// file (the lazy migration completes at the first save).
    pub fn set(&self, adapter: &str, fields: BTreeMap<String, String>) -> Result<(), String> {
        let schema = schema();
        let Some(a) = schema.iter().find(|s| s.adapter == adapter) else {
            return Err(format!("unknown adapter: {adapter}"));
        };
        for k in fields.keys() {
            if !a.fields.iter().any(|f| f.name == *k) {
                return Err(format!("unknown field for {adapter}: {k}"));
            }
        }
        let secret_names: Vec<&str> = a
            .fields
            .iter()
            .filter(|f| f.secret)
            .map(|f| f.name)
            .collect();
        if let Some(store) = &self.store {
            for (k, v) in &fields {
                if !secret_names.contains(&k.as_str()) {
                    continue;
                }
                let account = format!("{adapter}.{k}");
                if v.is_empty() {
                    store.delete(&account)?; // explicit clear
                } else {
                    store.set(&account, v)?;
                }
            }
        }
        let mut data = self.load();
        let entry = data.adapters.entry(adapter.to_string()).or_default();
        for (k, v) in fields {
            if secret_names.contains(&k.as_str()) && self.store.is_some() {
                // The keychain owns this value now — the file copy (if any
                // legacy plaintext survived) is scrubbed, never shadowed.
                entry.remove(&k);
            } else if v.is_empty() {
                entry.remove(&k);
            } else {
                entry.insert(k, v);
            }
        }
        if entry.is_empty() {
            data.adapters.remove(adapter);
        }
        data.secret_mode = Some(self.mode().to_string());
        self.save(&data)
    }

    /// Idempotent — deleting an absent adapter is honest Ok. Keychain
    /// entries for the adapter's secret fields are removed too (absent =
    /// fine).
    pub fn delete(&self, adapter: &str) -> Result<(), String> {
        if let Some(store) = &self.store {
            if let Some(a) = schema().iter().find(|s| s.adapter == adapter) {
                for f in &a.fields {
                    if f.secret {
                        let _ = store.delete(&format!("{adapter}.{}", f.name));
                    }
                }
            }
        }
        let mut data = self.load();
        data.adapters.remove(adapter);
        data.secret_mode = Some(self.mode().to_string());
        self.save(&data)
    }

    /// Map stored fields to the env names the core constructors read.
    /// Returns how many variables were set; a variable the process already
    /// has is NEVER overwritten (env wins — the #69 precedence rule).
    pub fn apply_env(&self) -> usize {
        let data = self.load();
        let direct: &[(&str, &str)] = &[
            ("s3_multipart.access_key", "IUM_S3_ACCESS_KEY"),
            ("s3_multipart.secret_key", "IUM_S3_SECRET_KEY"),
            ("s3_multipart.region", "IUM_S3_REGION"),
            ("terabox.ndus", "IUM_TERABOX_NDUS"),
            ("terabox.app_id", "IUM_TERABOX_APP_ID"),
            ("terabox.base", "IUM_TERABOX_BASE"),
            ("mega.user", "IUM_MEGA_USER"),
            ("mega.pass", "IUM_MEGA_PASS"),
            ("mega.twofa", "IUM_MEGA_2FA"),
            ("mega.bucket", "IUM_MEGA_BUCKET"),
            ("rclone_bridge.remote", "IUM_RCLONE_REMOTE"),
            ("rclone_bridge.mode", "IUM_RCLONE_MODE"),
            ("rclone_bridge.bucket", "IUM_RCLONE_BUCKET"),
            ("rclone_bridge.bin", "IUM_RCLONE_BIN"),
            ("feishu.app_id", "IUM_FEISHU_APP_ID"),
            ("feishu.app_secret", "IUM_FEISHU_APP_SECRET"),
            ("webdav.url", "IUM_WEBDAV_URL"),
            ("webdav.username", "IUM_WEBDAV_USER"),
            ("webdav.password", "IUM_WEBDAV_PASS"),
            ("webdav.vendor", "IUM_WEBDAV_VENDOR"),
            ("webdav.bucket", "IUM_WEBDAV_BUCKET"),
            ("google_drive.token", "IUM_GOOGLE_DRIVE_TOKEN"),
            ("youtube.client_id", "IUM_GOOGLE_CLIENT_ID"),
            ("youtube.client_secret", "IUM_GOOGLE_CLIENT_SECRET"),
            ("youtube.token", "IUM_YOUTUBE_TOKEN"),
        ];
        let mut set = 0;
        for (key, var) in direct {
            let Some((adapter, field)) = key.split_once('.') else {
                continue;
            };
            if let Some(entry) = data.adapters.get(adapter) {
                if let Some(v) = entry.get(field) {
                    if std::env::var_os(var).is_none() {
                        std::env::set_var(var, v);
                        set += 1;
                    }
                }
            }
        }
        // The rclone power-user pairs: one K=V per line, K must start with
        // RCLONE_CONFIG_ — foreign keys are refused, never set.
        if let Some(entry) = data.adapters.get("rclone_bridge") {
            if let Some(extra) = entry.get("extra_env") {
                for line in extra.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((k, v)) = line.split_once('=') {
                        if k.starts_with("RCLONE_CONFIG_") && std::env::var_os(k).is_none() {
                            std::env::set_var(k, v);
                            set += 1;
                        }
                    }
                }
            }
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ium-vault-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tmpdir("roundtrip");
        let vault = Vault::open(&dir);
        let mut data = VaultData::default();
        data.adapters
            .insert("mega".into(), entry(&[("user", "u@x"), ("pass", "p")]));
        vault.save(&data).expect("save");
        let loaded = vault.load();
        assert_eq!(
            loaded.adapters.get("mega").unwrap().get("pass").unwrap(),
            "p"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_file_is_empty_not_error() {
        let dir = tmpdir("missing");
        let vault = Vault::open(&dir);
        let loaded = vault.load();
        assert!(loaded.adapters.is_empty(), "a fresh machine has no vault");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_vault_is_empty_not_panic() {
        let dir = tmpdir("corrupt");
        std::fs::write(dir.join(VAULT_FILE), "{not json").unwrap();
        let vault = Vault::open(&dir);
        assert!(vault.load().adapters.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_0600_and_no_tmp_leftovers() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmpdir("perms");
        let vault = Vault::open(&dir);
        vault
            .save(&VaultData {
                adapters: BTreeMap::new(),
                secret_mode: None,
            })
            .expect("save");
        let meta = std::fs::metadata(dir.join(VAULT_FILE)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600, "0600 or bust");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != VAULT_FILE)
            .collect();
        assert!(
            leftovers.is_empty(),
            "atomic write leaves no tmp: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_merges_partially_and_empty_deletes() {
        let dir = tmpdir("merge");
        let vault = Vault::open(&dir);
        vault
            .set("terabox", entry(&[("ndus", "N1"), ("base", "https://b")]))
            .expect("set");
        // Partial merge: an absent field keeps its value…
        vault.set("terabox", entry(&[("ndus", "N2")])).expect("set");
        let got = vault.load();
        let tb = got.adapters.get("terabox").unwrap();
        assert_eq!(tb.get("ndus").unwrap(), "N2");
        assert_eq!(tb.get("base").unwrap(), "https://b");
        // …and an EMPTY value deletes the field.
        vault.set("terabox", entry(&[("base", "")])).expect("clear");
        let tb = vault.load().adapters.get("terabox").unwrap().clone();
        assert!(!tb.contains_key("base"), "empty value = delete, got {tb:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_whole_adapter() {
        let dir = tmpdir("delete");
        let vault = Vault::open(&dir);
        vault.set("mega", entry(&[("user", "u")])).expect("set");
        vault.delete("mega").expect("delete");
        assert!(!vault.load().adapters.contains_key("mega"));
        // Deleting an absent adapter is honest Ok — idempotent.
        vault.delete("never-was").expect("idempotent delete");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_env_maps_every_adapter_to_its_constructor_names() {
        let dir = tmpdir("apply");
        let vault = Vault::open(&dir);
        let mut data = VaultData::default();
        data.adapters.insert(
            "s3_multipart".into(),
            entry(&[
                ("access_key", "AK"),
                ("secret_key", "SK"),
                ("region", "us-east-1"),
            ]),
        );
        data.adapters
            .insert("terabox".into(), entry(&[("ndus", "ND")]));
        data.adapters
            .insert("mega".into(), entry(&[("user", "MU"), ("pass", "MP")]));
        data.adapters.insert(
            "feishu".into(),
            entry(&[("app_id", "FA"), ("app_secret", "FS")]),
        );
        vault.save(&data).unwrap();
        let n = vault.apply_env();
        assert_eq!(n, 8, "3 s3 + 1 terabox + 2 mega + 2 feishu, got {n}");
        assert_eq!(std::env::var("IUM_S3_ACCESS_KEY").unwrap(), "AK");
        assert_eq!(std::env::var("IUM_S3_SECRET_KEY").unwrap(), "SK");
        assert_eq!(std::env::var("IUM_S3_REGION").unwrap(), "us-east-1");
        assert_eq!(std::env::var("IUM_TERABOX_NDUS").unwrap(), "ND");
        assert_eq!(std::env::var("IUM_MEGA_USER").unwrap(), "MU");
        assert_eq!(std::env::var("IUM_MEGA_PASS").unwrap(), "MP");
        assert_eq!(std::env::var("IUM_FEISHU_APP_SECRET").unwrap(), "FS");
        // env wins: apply again — count reflects only previously-unset vars.
        let n2 = vault.apply_env();
        assert_eq!(n2, 0, "a second apply sets nothing (env wins)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_env_sets_rclone_extra_pairs_and_refuses_foreign_keys() {
        let dir = tmpdir("rclone");
        let vault = Vault::open(&dir);
        vault
            .set(
                "rclone_bridge",
                entry(&[
                    ("remote", "dropbox:"),
                    ("extra_env", "RCLONE_CONFIG_DROPBOX_TOKEN=abc\nBAD_KEY=nope"),
                ]),
            )
            .expect("set");
        let n = vault.apply_env();
        assert_eq!(std::env::var("IUM_RCLONE_REMOTE").unwrap(), "dropbox:");
        assert_eq!(std::env::var("RCLONE_CONFIG_DROPBOX_TOKEN").unwrap(), "abc");
        assert!(
            std::env::var("BAD_KEY").is_err(),
            "non-RCLONE_CONFIG_ keys are refused"
        );
        assert_eq!(n, 2, "remote + the one legal pair");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_covers_every_constructor_surface_and_masks_secrets() {
        let schema = schema();
        let names: Vec<&str> = schema.iter().map(|a| a.adapter).collect();
        for want in [
            "s3_multipart",
            "terabox",
            "mega",
            "rclone_bridge",
            "feishu",
            "webdav",
            "google_drive",
            "youtube",
        ] {
            assert!(names.contains(&want), "schema misses {want}: {names:?}");
        }
        let s3 = schema.iter().find(|a| a.adapter == "s3_multipart").unwrap();
        let secret_key = s3.fields.iter().find(|f| f.name == "secret_key").unwrap();
        assert!(secret_key.secret, "secret_key must be masked");
        let region = s3.fields.iter().find(|f| f.name == "region").unwrap();
        assert!(!region.secret, "region is not a secret");
        // Every schema field maps somewhere: no orphan fields.
        let all: Vec<&str> = schema
            .iter()
            .flat_map(|a| a.fields.iter().map(|f| f.name))
            .collect();
        assert!(all.contains(&"ndus") && all.contains(&"remote") && all.contains(&"app_secret"));
    }

    /// #83 — the WebDAV lane is the focused credential sheet (the protocol
    /// has no OAuth; the owner's own framing pinned that). url/username/
    /// password required, password masked, vendor optional.
    #[test]
    fn webdav_lane_is_the_credential_sheet() {
        let schema = schema();
        let w = schema
            .iter()
            .find(|a| a.adapter == "webdav")
            .expect("the webdav lane must exist in the schema");
        let field = |n: &str| {
            w.fields
                .iter()
                .find(|f| f.name == n)
                .unwrap_or_else(|| panic!("the webdav lane misses the {n} field"))
        };
        assert!(field("url").required, "the DAV root URL is required");
        assert!(field("username").required, "the username is required");
        let pass = field("password");
        assert!(
            pass.required && pass.secret,
            "the password is a masked, required secret"
        );
        assert!(
            !field("vendor").required,
            "vendor is optional — generic DAV is the default"
        );
        // The env mapping exists for every webdav field (no orphans):
        // one apply_env run sets all five names.
        let mapped: &[&str] = &[
            "IUM_WEBDAV_URL",
            "IUM_WEBDAV_USER",
            "IUM_WEBDAV_PASS",
            "IUM_WEBDAV_VENDOR",
            "IUM_WEBDAV_BUCKET",
        ];
        let dir = tmpdir("webdav-map");
        let v = Vault::open(&dir);
        let mut fields = BTreeMap::new();
        fields.insert("url".to_string(), "https://dav.example.com".to_string());
        fields.insert("username".to_string(), "u".to_string());
        fields.insert("password".to_string(), "p".to_string());
        fields.insert("vendor".to_string(), "nextcloud".to_string());
        fields.insert("bucket".to_string(), "ium".to_string());
        v.set("webdav", fields).unwrap();
        let n = v.apply_env();
        for var in mapped {
            assert!(std::env::var(var).is_ok(), "apply_env must map {var}");
            std::env::remove_var(var); // leave the shared process env clean
        }
        assert_eq!(n, 5, "the five webdav fields map to env vars");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #84 — the Google Drive lane is ONE hidden token row: the browser
    /// sign-in writes it inside the process, the UI renders no input, and
    /// its presence IS the connection state. The apply_env mapping hands
    /// the token to the core constructor (`IUM_GOOGLE_DRIVE_TOKEN`).
    #[test]
    fn google_drive_lane_is_one_hidden_token_row() {
        let schema = schema();
        let g = schema
            .iter()
            .find(|a| a.adapter == "google_drive")
            .expect("the google_drive lane must exist in the schema");
        assert_eq!(g.fields.len(), 1, "no field wall — the sign-in IS the lane");
        let token = &g.fields[0];
        assert_eq!(token.name, "token");
        assert!(
            token.required,
            "presence is the state — the row must be required"
        );
        assert!(token.secret, "the token is a keychain secret (#81)");
        assert!(token.hidden, "no input renders — the sign-in writes it");
        // The mapping hands the token to the core constructor.
        let dir = tmpdir("gdrive-map");
        let v = Vault::open(&dir);
        let mut fields = BTreeMap::new();
        fields.insert("token".to_string(), "{\"type\":\"drive\"}".to_string());
        v.set("google_drive", fields).unwrap();
        let n = v.apply_env();
        assert!(
            std::env::var("IUM_GOOGLE_DRIVE_TOKEN").is_ok(),
            "apply_env must map the token"
        );
        std::env::remove_var("IUM_GOOGLE_DRIVE_TOKEN"); // leave the shared env clean
        assert_eq!(n, 1, "the one token maps to one env var");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #85 — the YouTube lane: two visible client fields (the honest BYO
    /// setup until IUM ships a registered client) + ONE hidden token row
    /// the loopback sign-in writes. apply_env maps all three.
    #[test]
    fn youtube_lane_is_two_fields_and_a_hidden_token() {
        let schema = schema();
        let y = schema
            .iter()
            .find(|a| a.adapter == "youtube")
            .expect("the youtube lane must exist in the schema");
        let field = |n: &str| {
            y.fields
                .iter()
                .find(|f| f.name == n)
                .unwrap_or_else(|| panic!("the youtube lane misses the {n} field"))
        };
        assert!(field("client_id").required && !field("client_id").secret);
        assert!(field("client_secret").required && field("client_secret").secret);
        let token = field("token");
        assert!(token.required && token.secret && token.hidden);
        assert_eq!(
            y.fields.len(),
            3,
            "two client fields + the hidden token — nothing else"
        );
        // apply_env maps all three names.
        let dir = tmpdir("youtube-map");
        let v = Vault::open(&dir);
        let mut fields = BTreeMap::new();
        fields.insert("client_id".to_string(), "cid".to_string());
        fields.insert("client_secret".to_string(), "sec".to_string());
        fields.insert("token".to_string(), "{\"access_token\":\"a\"}".to_string());
        v.set("youtube", fields).unwrap();
        let n = v.apply_env();
        for var in [
            "IUM_GOOGLE_CLIENT_ID",
            "IUM_GOOGLE_CLIENT_SECRET",
            "IUM_YOUTUBE_TOKEN",
        ] {
            assert!(std::env::var(var).is_ok(), "apply_env must map {var}");
            std::env::remove_var(var); // leave the shared env clean
        }
        assert_eq!(n, 3, "the three youtube fields map to env vars");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- #81: the SecretStore seam (keychain mode vs the honest fallback) ----

    /// In-memory keychain double — deterministic everywhere, including CI.
    #[derive(Default)]
    struct MemoryStore(std::sync::Mutex<BTreeMap<String, String>>);

    impl SecretStore for MemoryStore {
        fn set(&self, account: &str, value: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .insert(account.to_string(), value.to_string());
            Ok(())
        }
        fn get(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }
        fn delete(&self, account: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn keychain_vault(tag: &str) -> (std::path::PathBuf, Vault) {
        let dir = tmpdir(tag);
        let v = Vault::open_keychain(&dir, Some(Box::new(MemoryStore::default())));
        assert_eq!(v.mode(), MODE_KEYRING);
        (dir, v)
    }

    #[test]
    fn keychain_mode_never_writes_secret_values_to_the_file() {
        let (dir, v) = keychain_vault("kr");
        let mut fields = BTreeMap::new();
        fields.insert("access_key".to_string(), "AK".to_string());
        fields.insert("secret_key".to_string(), "SK".to_string());
        fields.insert("region".to_string(), "us".to_string());
        v.set("s3_multipart", fields).unwrap();

        let file = std::fs::read_to_string(dir.join(VAULT_FILE)).unwrap();
        assert!(file.contains(MODE_KEYRING), "the marker must say keyring");
        assert!(
            !file.contains("SK") && !file.contains("AK"),
            "secret values must NEVER land in the file (access_key is a secret in \
             the schema too), got: {file}"
        );
        assert!(file.contains("us"), "non-secrets stay in the file");

        // load() reassembles the full view (secrets from the seam, masked
        // later at the IPC boundary — values never reach the UI).
        let data = v.load();
        assert_eq!(data.adapters["s3_multipart"]["secret_key"], "SK");
        assert_eq!(data.adapters["s3_multipart"]["access_key"], "AK");

        // Explicit clear removes from the seam too.
        let mut clear = BTreeMap::new();
        clear.insert("secret_key".to_string(), String::new());
        v.set("s3_multipart", clear).unwrap();
        let data = v.load();
        assert!(
            !data.adapters["s3_multipart"].contains_key("secret_key"),
            "empty value = delete, keychain included"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_mode_is_the_honest_fallback_with_a_marker() {
        let dir = tmpdir("fb");
        let v = Vault::open_keychain(&dir, None); // no secret service available
        assert_eq!(v.mode(), MODE_FILE);
        let mut fields = BTreeMap::new();
        fields.insert("access_key".to_string(), "AK".to_string());
        fields.insert("secret_key".to_string(), "SK".to_string());
        v.set("s3_multipart", fields).unwrap();

        let file = std::fs::read_to_string(dir.join(VAULT_FILE)).unwrap();
        assert!(file.contains(MODE_FILE), "the marker must say file");
        assert!(
            file.contains("SK"),
            "the fallback keeps today's behavior — secrets in the 0600 file, marked"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_plaintext_migrates_at_the_first_save() {
        let dir = tmpdir("mig");
        // Seed the file the pre-#81 way (plaintext secret era).
        Vault::open(&dir)
            .set("s3_multipart", {
                let mut m = BTreeMap::new();
                m.insert("secret_key".to_string(), "OLD".to_string());
                m.insert("region".to_string(), "us".to_string());
                m
            })
            .unwrap();

        // Same dir, keychain mode — the product upgrade path.
        let v = Vault::open_keychain(&dir, Some(Box::new(MemoryStore::default())));
        assert_eq!(v.mode(), MODE_KEYRING);
        // Reads still serve the legacy value — nothing breaks, nothing lost.
        let data = v.load();
        assert_eq!(data.adapters["s3_multipart"]["secret_key"], "OLD");
        assert_eq!(data.adapters["s3_multipart"]["region"], "us");

        // The first save moves the secret to the keychain and scrubs BOTH
        // the old and the new plaintext from the file.
        let mut fields = BTreeMap::new();
        fields.insert("secret_key".to_string(), "NEW".to_string());
        v.set("s3_multipart", fields).unwrap();
        let file = std::fs::read_to_string(dir.join(VAULT_FILE)).unwrap();
        assert!(!file.contains("OLD"), "legacy plaintext scrubbed");
        assert!(!file.contains("NEW"), "new plaintext never written");
        let data = v.load();
        assert_eq!(data.adapters["s3_multipart"]["secret_key"], "NEW");
        assert_eq!(data.adapters["s3_multipart"]["region"], "us", "kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_clears_the_keychain_entries_too() {
        let (dir, v) = keychain_vault("del");
        let mut fields = BTreeMap::new();
        fields.insert("app_id".to_string(), "FA".to_string());
        fields.insert("app_secret".to_string(), "FS".to_string());
        v.set("feishu", fields).unwrap();
        v.delete("feishu").unwrap();
        let data = v.load();
        assert!(!data.adapters.contains_key("feishu"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
