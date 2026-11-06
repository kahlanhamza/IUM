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
    ]
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug)]
pub struct VaultData {
    pub adapters: BTreeMap<String, BTreeMap<String, String>>,
}

pub struct Vault {
    path: PathBuf,
}

impl Vault {
    pub fn open(dir: &Path) -> Self {
        Self {
            path: dir.join(VAULT_FILE),
        }
    }

    /// A missing file is an empty vault, not an error; a corrupted file is
    /// an empty vault too — credentials are re-enterable, panic is not an
    /// option in a startup path.
    pub fn load(&self) -> VaultData {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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

    /// Partial merge: absent field = keep, empty value = delete.
    pub fn set(&self, adapter: &str, fields: BTreeMap<String, String>) -> Result<(), String> {
        let mut data = self.load();
        let entry = data.adapters.entry(adapter.to_string()).or_default();
        for (k, v) in fields {
            if v.is_empty() {
                entry.remove(&k);
            } else {
                entry.insert(k, v);
            }
        }
        if entry.is_empty() {
            data.adapters.remove(adapter);
        }
        self.save(&data)
    }

    /// Idempotent — deleting an absent adapter is honest Ok.
    pub fn delete(&self, adapter: &str) -> Result<(), String> {
        let mut data = self.load();
        data.adapters.remove(adapter);
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
        for want in ["s3_multipart", "terabox", "mega", "rclone_bridge", "feishu"] {
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
}
