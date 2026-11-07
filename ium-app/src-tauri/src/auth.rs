//! The browser sign-in's pure core (#82): `rclone authorize <backend>`
//! opens the provider's real sign-in page in the user's browser; they
//! sign in as normal (2FA and consent screens included — the provider
//! owns the flow); rclone prints the resulting token config; this module
//! extracts it, builds the env lines the rclone-bridge lane consumes, and
//! merges them into the stored block WITHOUT the UI ever seeing a stored
//! secret value.
//!
//! Pure-std on purpose (the #61 contract): zero dependencies, so the
//! local `rustc --test auth.rs` harness stays honest, while the embedding
//! crate takes its compile proof from CI.
//!
//! The allowlist is the injection contract: only the obvious OAuth
//! backends may pass through to a spawned command line — a typed string
//! never becomes an argument. Google DRIVE rides the same rclone authorize
//! flow since the owner lifted the shelving (#84 → #59); YouTube has NO
//! rclone backend and rides its own native flow (#85).

/// The obvious OAuth backends — an allowlist, not a free-form field. The
/// TS picker renders exactly this list (`connect.ts` enforces the same
/// set); the shell refuses anything else before spawning. Google DRIVE
/// joined when the owner lifted the shelving (#84); YouTube stays OUT —
/// rclone has no such backend, its sign-in is #85's own loopback flow.
pub fn is_oauth_backend(id: &str) -> bool {
    matches!(id, "pcloud" | "dropbox" | "onedrive" | "box" | "drive")
}

/// The remote name a browser-signed-in backend rides under: `IUM_<BACKEND>`
/// uppercased, non-alphanumerics folded to `_` (rclone's own env-config
/// practice: `RCLONE_CONFIG_<NAME>_<KEY>`).
pub fn remote_name(backend: &str) -> String {
    let mut out = String::from("IUM_");
    for c in backend.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    out
}

/// The env lines the bridge lane consumes for a browser-signed-in
/// backend: the remote's type plus its token, in rclone's env-config
/// spelling. The token value rides `extra_env` — a secret field, so the
/// OS keychain stores it like every other secret (#81's store contract).
pub fn env_lines(backend: &str, token_json: &str) -> Vec<(String, String)> {
    let name = remote_name(backend);
    vec![
        (format!("RCLONE_CONFIG_{name}_TYPE"), backend.to_string()),
        (
            format!("RCLONE_CONFIG_{name}_TOKEN"),
            token_json.to_string(),
        ),
    ]
}

/// Extract the token JSON from `rclone authorize`'s stdout. rclone's
/// shape: a marker line ending in `--->`, the JSON object, `<---End
/// paste`. Tolerant of version drift: with no markers present, the one
/// brace-delimited line naming `type` and `token` wins. Refusals are
/// honest sentences, not codes — the sheet renders them as-is.
pub fn parse_authorize_output(out: &str) -> Result<String, String> {
    let marked = out.contains("--->");
    let mut in_block = false;
    let mut candidates: Vec<&str> = Vec::new();
    for line in out.lines() {
        let l = line.trim();
        if l.ends_with("--->") {
            in_block = true;
            continue;
        }
        if l.starts_with("<---") {
            in_block = false;
            continue;
        }
        let eligible = if marked { in_block } else { true };
        if eligible
            && l.starts_with('{')
            && l.ends_with('}')
            && l.contains("\"type\"")
            && l.contains("\"token\"")
        {
            candidates.push(l);
        }
    }
    candidates.dedup(); // identical repeats are one line; distinct repeats are ambiguity
    match candidates.len() {
        0 => Err("the browser sign-in finished without a token — no config line appeared.".into()),
        1 => Ok(candidates[0].to_string()),
        _ => Err("the sign-in output was ambiguous — more than one config line appeared.".into()),
    }
}

/// Merge `KEY=VALUE` lines into an existing block: the LAST value for a
/// key wins, first-appearance order survives, blank lines drop. The
/// bridge lane's `extra_env` is one multi-line secret — a browser sign-in
/// must update its token WITHOUT the UI reading the stored value (masked
/// by contract, #69), so the merge runs shell-side, here.
pub fn merge_env_lines(existing: &str, add: &[String]) -> String {
    let added = add.join("\n");
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for block in [existing, added.as_str()] {
        for line in block.lines() {
            let l = line.trim();
            if l.is_empty() || !l.contains('=') {
                continue;
            }
            let (k, v) = l.split_once('=').expect("contains '=' above");
            if !map.contains_key(k) {
                order.push(k.to_string());
            }
            map.insert(k.to_string(), v.to_string());
        }
    }
    order
        .iter()
        .map(|k| format!("{k}={}", map[k]))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCLOUD_OK: &str = "\
If your browser doesn't open automatically go to the following link: http://127.0.0.1:53682/auth?state=xyz
Log in and authorize rclone for access.
Waiting for code...
Got code.
Paste the following into your remote machine --->
{\"type\":\"pcloud\",\"token\":{\"access_token\":\"a.b\",\"token_type\":\"bearer\",\"refresh_token\":\"r\",\"expiry\":\"2026-01-01T00:00:00Z\"}}
<---End paste
";

    #[test]
    fn parses_the_real_rclone_output() {
        let t = parse_authorize_output(PCLOUD_OK).expect("the fixture must parse");
        assert!(t.starts_with("{\"type\":\"pcloud\""));
        assert!(t.contains("\"access_token\""));
        assert!(t.contains("\"refresh_token\""));
    }

    #[test]
    fn tolerates_version_drift_without_markers() {
        let out = "Got code.\n{\"type\":\"box\",\"token\":{\"access_token\":\"z\"}}\n";
        assert_eq!(
            parse_authorize_output(out).unwrap(),
            "{\"type\":\"box\",\"token\":{\"access_token\":\"z\"}}"
        );
    }

    #[test]
    fn a_cancelled_signin_has_no_token() {
        let out = "If your browser doesn't open automatically go to the following link: http://127.0.0.1:53682/auth?state=xyz\nWaiting for code...\n";
        let e = parse_authorize_output(out).unwrap_err();
        assert!(e.contains("without a token"));
    }

    #[test]
    fn garbage_is_refused() {
        assert!(parse_authorize_output("hello world\nno json here\n").is_err());
    }

    #[test]
    fn a_line_without_type_or_token_is_not_a_candidate() {
        let out = "{\"foo\":1}\n{\"type\":\"box\",\"token\":{\"access_token\":\"z\"}}\n";
        assert!(parse_authorize_output(out)
            .unwrap()
            .contains("\"type\":\"box\""));
    }

    #[test]
    fn ambiguous_output_is_refused() {
        let out = "\
--->
{\"type\":\"box\",\"token\":{\"access_token\":\"a\"}}
<---End paste
--->
{\"type\":\"box\",\"token\":{\"access_token\":\"b\"}}
<---End paste
";
        let e = parse_authorize_output(out).unwrap_err();
        assert!(e.contains("ambiguous"));
    }

    #[test]
    fn env_lines_follow_rclone_env_config_spelling() {
        let lines = env_lines("pcloud", "{}");
        assert_eq!(lines[0].0, "RCLONE_CONFIG_IUM_PCLOUD_TYPE");
        assert_eq!(lines[0].1, "pcloud");
        assert_eq!(lines[1].0, "RCLONE_CONFIG_IUM_PCLOUD_TOKEN");
        assert_eq!(lines[1].1, "{}");
    }

    #[test]
    fn the_name_folds_non_alphanumerics() {
        assert_eq!(remote_name("one-drive"), "IUM_ONE_DRIVE");
    }

    #[test]
    fn the_allowlist_is_the_obvious_oauth_ones_drive_included() {
        for id in ["pcloud", "dropbox", "onedrive", "box", "drive"] {
            assert!(is_oauth_backend(id));
        }
        // YouTube is NOT here — rclone has no such backend; its sign-in is
        // #85's own loopback flow, never a free-form spawn id.
        assert!(!is_oauth_backend("youtube"));
        assert!(!is_oauth_backend(""));
        assert!(!is_oauth_backend("$(x)"));
    }

    #[test]
    fn drive_env_lines_follow_the_ium_drive_remote() {
        let lines = env_lines("drive", "{}");
        assert_eq!(lines[0].0, "RCLONE_CONFIG_IUM_DRIVE_TYPE");
        assert_eq!(lines[0].1, "drive");
        assert_eq!(lines[1].0, "RCLONE_CONFIG_IUM_DRIVE_TOKEN");
        assert_eq!(lines[1].1, "{}");
    }

    #[test]
    fn merge_adds_new_keys_and_preserves_order() {
        let merged = merge_env_lines(
            "RCLONE_CONFIG_MEGA_TYPE=mega\nRCLONE_CONFIG_MEGA_USER=u@x",
            &[
                "RCLONE_CONFIG_IUM_PCLOUD_TYPE=pcloud".into(),
                "RCLONE_CONFIG_IUM_PCLOUD_TOKEN={\"t\":1}".into(),
            ],
        );
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "RCLONE_CONFIG_MEGA_TYPE=mega");
        assert_eq!(lines[2], "RCLONE_CONFIG_IUM_PCLOUD_TYPE=pcloud");
        assert_eq!(lines[3], "RCLONE_CONFIG_IUM_PCLOUD_TOKEN={\"t\":1}");
    }

    #[test]
    fn merge_replaces_a_stale_token_last_value_wins() {
        let merged = merge_env_lines(
            "RCLONE_CONFIG_IUM_BOX_TOKEN={\"old\":1}\nA=B",
            &["RCLONE_CONFIG_IUM_BOX_TOKEN={\"new\":2}".into()],
        );
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(
            lines,
            vec!["RCLONE_CONFIG_IUM_BOX_TOKEN={\"new\":2}", "A=B"]
        );
    }

    #[test]
    fn merge_drops_blank_lines_and_keyless_junk() {
        let merged = merge_env_lines("\nnot-a-pair\nKEEP=1\n", &[]);
        assert_eq!(merged, "KEEP=1");
    }
}
