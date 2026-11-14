//! The bundled rclone's path resolution (#118): pure-std, zero deps, the
//! same contract shape as `auth.rs` — a standalone `rustc --test` harness
//! stays honest locally while the embedding crate takes its compile proof
//! from CI.
//!
//! Why this module exists: every bridge-family lane (MEGA, WebDAV, Google
//! Drive, the generic rclone bridge) and every rclone `authorize` sign-in
//! spawns an external rclone binary. The default used to be bare `rclone`
//! from PATH — and the installers shipped none, so a clean machine had
//! EVERY lane dead while its owner had typed perfectly valid credentials
//! (the owner's wire report: "I added mega with my email/password, nothing
//! is working"). Now the installers ship rclone as a Tauri sidecar
//! (`bundle.externalBin`), which lands NEXT TO the app executable in every
//! packaging: NSIS/MSI (`%LOCALAPPDATA%\IUM\rclone.exe`), deb/rpm
//! (`/usr/bin/`), AppImage (`squashfs-root/usr/bin/`), DMG
//! (`Contents/MacOS/`), and `tauri dev`'s target dir alike. Resolution is
//! precedence-ordered, never guessed:

/// The env vars the core bridge constructors read for the binary path —
/// the sidecar install writes the resolved path into each of them
/// (only-if-unset: real env and the vault's `bin` field always win).
pub const RCLONE_BIN_ENVS: [&str; 4] = [
    "IUM_RCLONE_BIN",
    "IUM_MEGA_RCLONE_BIN",
    "IUM_WEBDAV_RCLONE_BIN",
    "IUM_GOOGLE_DRIVE_RCLONE_BIN",
];

/// The sidecar's on-disk name next to the running executable.
pub const SIDECAR_NAME: &str = if cfg!(target_os = "windows") {
    "rclone.exe"
} else {
    "rclone"
};

/// The last-resort default (unchanged from the pre-sidecar world): a
/// machine whose operator installed rclone themselves still works, and a
/// machine with neither sidecar nor PATH still gets the honest "could not
/// run rclone" refusal — never a guessed path.
pub const PATH_FALLBACK: &str = "rclone";

/// The resolution order, pure so the harness can prove it:
/// 1. EXPLICIT — the vault's `bin` field (the operator pointed somewhere);
/// 2. ENV — `IUM_RCLONE_BIN` (the vault fills this from the same field, so
///    this is the same seat, reached through the env contract);
/// 3. SIDECAR — the binary bundled next to the running executable;
/// 4. PATH — `rclone`, the pre-sidecar default.
pub fn resolve_rclone_bin(
    explicit: Option<&str>,
    env_bin: Option<&str>,
    sidecar: Option<&str>,
) -> String {
    for seat in [explicit, env_bin, sidecar] {
        if let Some(v) = seat {
            let t = v.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    PATH_FALLBACK.to_string()
}

/// The real sidecar lookup: next to THIS process's executable.
pub fn sidecar_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let p = dir.join(SIDECAR_NAME);
    if p.is_file() {
        Some(p.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// The startup install (#118): the resolved sidecar path becomes the
/// default bin for every bridge family, through the env vars core reads —
/// only-if-unset, so real env vars and the vault's `bin` field always win.
/// Called once in `run()` BEFORE the vault fills the env gaps, so the
/// `*_from_env` constructors see `IUM_*_RCLONE_BIN` even on a machine
/// whose vault has no `bin` field saved.
pub fn install_rclone_env() -> bool {
    let Some(p) = sidecar_path() else {
        return false;
    };
    let mut any = false;
    for k in RCLONE_BIN_ENVS {
        let unset = std::env::var(k)
            .map(|v| v.trim().is_empty())
            .unwrap_or(true);
        if unset {
            std::env::set_var(k, &p);
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #118: the precedence order — explicit wins, env second, sidecar
    /// third, PATH last. Every seat's truth: an operator who pointed the
    /// lane somewhere is obeyed; a bundled machine works out of the box;
    /// a machine with a hand-installed rclone still works.
    #[test]
    fn the_resolution_order_is_explicit_env_sidecar_path() {
        assert_eq!(
            resolve_rclone_bin(
                Some("/opt/my/rclone"),
                Some("/env/rclone"),
                Some("/side/rclone")
            ),
            "/opt/my/rclone",
            "the vault's bin field outranks everything"
        );
        assert_eq!(
            resolve_rclone_bin(Some("  "), Some("/env/rclone"), Some("/side/rclone")),
            "/env/rclone",
            "an empty/blank explicit value is no explicit value"
        );
        assert_eq!(
            resolve_rclone_bin(None, Some("/env/rclone"), Some("/side/rclone")),
            "/env/rclone",
            "env outranks the sidecar (the vault rides it)"
        );
        assert_eq!(
            resolve_rclone_bin(None, None, Some("/side/rclone")),
            "/side/rclone",
            "the bundled sidecar outranks PATH"
        );
        assert_eq!(
            resolve_rclone_bin(None, None, None),
            PATH_FALLBACK,
            "nothing found: PATH, the pre-sidecar default"
        );
    }

    /// #118: the sidecar's on-disk name follows the OS — Windows ships
    /// `rclone.exe`, every other packaging keeps `rclone`.
    #[test]
    fn the_sidecar_name_follows_the_os() {
        if cfg!(target_os = "windows") {
            assert_eq!(SIDECAR_NAME, "rclone.exe");
        } else {
            assert_eq!(SIDECAR_NAME, "rclone");
        }
    }

    /// #118: the env install covers exactly the four vars core's bridge
    /// constructors read — one var per family, no more, no less.
    #[test]
    fn the_env_install_targets_the_core_constructor_vars() {
        assert_eq!(
            RCLONE_BIN_ENVS,
            [
                "IUM_RCLONE_BIN",
                "IUM_MEGA_RCLONE_BIN",
                "IUM_WEBDAV_RCLONE_BIN",
                "IUM_GOOGLE_DRIVE_RCLONE_BIN",
            ]
        );
    }
}
