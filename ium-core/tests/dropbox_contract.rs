//! #109 — native Dropbox adapter contract.
//!
//! RED at HEAD: `ium_core::dropbox` does not exist yet; the compile failure of
//! this test target IS the RED evidence (the `verify_is_required.rs`
//! contract pattern). After the adapter lands this target compiles and all
//! assertions pass.
//!
//! Pins from the #109 design note:
//! * `REDIRECT_PORT = 8738` (fixed loopback port, Google lane convention 8737).
//! * `redirect_uri()` == `http://localhost:8738` (registered exactly, port is
//!   part of the exact match).
//! * `content_hash` matches the official algorithm (per-4-MiB-block SHA-256,
//!   digests concatenated, SHA-256 of the concatenation, hex; empty input is
//!   `sha256("")`) — vectors cross-checked with Python `hashlib` (independent
//!   implementation, `scripts/gen_hash_vectors.py`).
//! * The adapter registers under the name `dropbox` and is upload-only.

use ium_core::adapter::TransferAdapter;
use ium_core::dropbox::{content_hash, redirect_uri, DropboxAdapter, REDIRECT_PORT};

/// data[i] = (i % 251) as u8 — mirrors scripts/gen_hash_vectors.py.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn redirect_port_is_fixed_8738() {
    assert_eq!(REDIRECT_PORT, 8738);
    assert_eq!(redirect_uri(), "http://localhost:8738");
}

#[test]
fn content_hash_matches_official_algorithm_vectors() {
    // empty input -> sha256("")
    assert_eq!(
        content_hash(&[]),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // sub-block (< 4 MiB)
    assert_eq!(
        content_hash(&pattern(1024 * 1024)),
        "004fbf44b4c0a58632164fc9ec00e8f35e330037fd4425dd947cf65d930762c2"
    );
    // exactly one 4 MiB block
    assert_eq!(
        content_hash(&pattern(4 * 1024 * 1024)),
        "b9654428408015906b44a00935b70af33830aa344b780b0eabd535a133150d04"
    );
    // one block + tail
    assert_eq!(
        content_hash(&pattern(5 * 1024 * 1024)),
        "b109747a45702e22475f0ae8a9056c7621fac74aad54f7bf80bdc6bb157d897f"
    );
    // two blocks + tail + 1 (odd tail length, exercises final partial block)
    assert_eq!(
        content_hash(&pattern(2 * 4 * 1024 * 1024 + 1024 * 1024 + 1)),
        "040a6fa5f007f1f3df8bd13fec85bcbfc26fe3a288b75c0e2f0bad4b9ce279a2"
    );
}

#[test]
fn adapter_registers_as_dropbox_and_is_resumable_only() {
    let cfg = ium_core::dropbox::DropboxConfig::from_parts(
        r#"{"access_token":"a","refresh_token":"r","expires_in":14400}"#,
    )
    .expect("a real token shape parses");
    let a = DropboxAdapter::new(cfg);
    assert_eq!(a.name(), "dropbox");
    let cap = a.capabilities();
    // Session-based resume yes; no dynamic chunks (prescribed geometry), no
    // tail-race (append_v2 is a strictly sequential-offset session resource).
    assert!(cap.has(ium_core::adapter::Capability::RESUMABLE));
    assert!(!cap.has(ium_core::adapter::Capability::DYNAMIC_CHUNKS));
    assert!(!cap.has(ium_core::adapter::Capability::TAIL_RACE));
    // prescribed geometry: uniform 4 MiB blocks (#56 channel).
    assert_eq!(a.uniform_block(), Some(4 * 1024 * 1024));
}
