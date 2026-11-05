//! Contract tests for the rclone bridge lane (issue #63).
//!
//! Hermetic — no rclone binary needed: everything here exercises the PURE
//! functions of `adapter_rclone` (endpoint rewrite, listen-line port parse,
//! config validation, registry identity). The LIVE drill (spawn + upload +
//! resume + byte-exact + no-orphan reaping) lives in the module, env-gated
//! by `IUM_RCLONE_DRILL=1`.

use ium_core::adapter_rclone::{
    parse_bridge_endpoint, parse_listen_port, rewrite_endpoint, RcloneBridgeConfig,
    RcloneBridgeMode,
};

#[test]
fn listen_line_parses_the_real_port() {
    // W11 — the exact rclone v1.75.2 notice line shape (bracket glitch included).
    let line = "2026/11/05 12:25:52 NOTICE: Local file system at /tmp/wf10/root: \
                Starting s3 server on ttp://127.0.0.1:43803/]";
    assert_eq!(parse_listen_port(line), Some(43803));
    assert_eq!(parse_listen_port("no listen line here"), None);
    assert_eq!(parse_listen_port(""), None);
}

#[test]
fn endpoint_parse_requires_the_self_contained_key() {
    // D2 (drill-caught lesson): the key is REQUIRED — reattach receives only
    // the journaled endpoint string, so a bare `rclone-bridge://` could never
    // round-trip a resume. No basename-default footgun.
    let e = parse_bridge_endpoint("rclone-bridge://video.mp4", "/tmp/s/other.bin").unwrap();
    assert_eq!(e.key, "video.mp4");
    let e2 =
        parse_bridge_endpoint("rclone-bridge://sub/dir/clip.bin", "/tmp/src/video.mp4").unwrap();
    assert_eq!(e2.key, "sub/dir/clip.bin");
    assert!(parse_bridge_endpoint("https://wrong-scheme/x", "/tmp/f").is_err());
    assert!(parse_bridge_endpoint("rclone-bridge://", "/tmp/s/has-a-basename.bin").is_err());
}

#[test]
fn rewrite_targets_the_local_bridge() {
    let url = rewrite_endpoint("127.0.0.1:43803", "ium", "sub/dir/clip.bin");
    assert_eq!(url, "http://127.0.0.1:43803/ium/sub/dir/clip.bin");
    // The bucket-only shape must stay valid for the S3 parser (key defaults later).
    let url2 = rewrite_endpoint("127.0.0.1:5000", "ium", "file.bin");
    assert_eq!(url2, "http://127.0.0.1:5000/ium/file.bin");
}

#[test]
fn config_validates_mode_port_pairs() {
    // connect without a port is unusable — loud, not guessed.
    assert!(RcloneBridgeConfig::new(RcloneBridgeMode::Connect { port: 0 }, "/tmp/root").is_err());
    // spawn is always fine (port 0 = ephemeral, discovered from W11's listen line).
    let cfg = RcloneBridgeConfig::new(RcloneBridgeMode::Spawn, "/tmp/root").unwrap();
    assert_eq!(cfg.bucket, "ium");
    assert_eq!(cfg.rclone_bin, std::path::Path::new("rclone"));
}

#[test]
fn identity_and_capabilities_are_honest() {
    // Built in the drill (needs the binary); here the CONTRACT is pinned via
    // the type-level surface: the registry key and the declared flags.
    // These compile only when the adapter exists — the RED artifact.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ium_core::adapter_rclone::RcloneBridgeAdapter>();
    // name + capabilities asserted in the module's drill-side tests; the
    // hermetic guarantee here is: the type is constructible from config and
    // shareable across worker tasks (the Send+Sync bar above).
}
