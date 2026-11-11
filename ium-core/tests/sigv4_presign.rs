//! SigV4 presigned GET URLs — the shareable-link lane (the destination
//! work, owner-ratified 2026-11-11). A finished upload must be able to
//! hand the user a link; for private buckets that means a query-string
//! signature (the presigned GET shape), expiring, never auto-publishing.
//!
//! RED artifact: at the pre-implementation HEAD this file FAILS TO
//! COMPILE — `presign_url` does not exist yet (the module only signs
//! headers for the upload path).
//!
//! The vectors below were computed by an INDEPENDENT Python reference
//! (`scripts/presign_reference.py`, pure stdlib, written fresh from the
//! AWS SigV4 presigned-URL rules — never translated from the Rust). The
//! reference is anchored: EVERY run re-derives the published AWS
//! developer-guide Vector B signature (`f0e8bdb8…6bdb41`, already pinned
//! in `sigv4.rs` from AWS's docs) with its own chain and REFUSES to mint
//! if it drifts — the HMAC chain and canonicalization are re-proven before
//! the presign values print. The access key is AWS's public documentation
//! example key (AKIAIOSFODNN7EXAMPLE), matching the #6 vectors.

use ium_core::sigv4::{Credentials, PresignInput, SigV4Signer};

const AK: &str = "AKIAIOSFODNN7EXAMPLE";
const SK: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

fn s3_signer() -> SigV4Signer {
    SigV4Signer::new(
        Credentials {
            access_key: AK.into(),
            secret_key: SK.into(),
        },
        "us-east-1",
        "s3",
    )
}

/// Vector P1 — the canonical presigned GET (independent reference V1).
/// Query order is the canonical byte-sort; the credential scope is
/// percent-encoded (`/` → `%2F`) exactly as the wire URL must carry it.
#[test]
fn vector_presigned_get_canonical_case() {
    let q = s3_signer()
        .presign_url(PresignInput {
            host: "examplebucket.s3.amazonaws.com",
            canonical_uri: "/test.txt",
            expires_secs: 86400,
            date: "20130524",
            datetime: "20130524T000000Z",
        })
        .expect("canonical expires is legal");
    assert_eq!(
        q,
        "X-Amz-Algorithm=AWS4-HMAC-SHA256&\
         X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&\
         X-Amz-Date=20130524T000000Z&\
         X-Amz-Expires=86400&\
         X-Amz-SignedHeaders=host&\
         X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
    );
}

/// Vector P2 — loopback + a pre-encoded object key (`$` → `%24`, the
/// caller-owned canonical URI; the module signs what it is given, the
/// adapter owns the exact path — same contract as header signing).
#[test]
fn vector_presigned_get_loopback_encoded_key() {
    let q = s3_signer()
        .presign_url(PresignInput {
            host: "127.0.0.1:9000",
            canonical_uri: "/ium-test/obj%24name",
            expires_secs: 3600,
            date: "20261111",
            datetime: "20261111T120000Z",
        })
        .expect("3600 s is legal");
    assert_eq!(
        q,
        "X-Amz-Algorithm=AWS4-HMAC-SHA256&\
         X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261111%2Fus-east-1%2Fs3%2Faws4_request&\
         X-Amz-Date=20261111T120000Z&\
         X-Amz-Expires=3600&\
         X-Amz-SignedHeaders=host&\
         X-Amz-Signature=c4785c04e9475190fbfdaa045553a020a28d5582f6927649b9c96e6ffe4aac92"
    );
}

/// Determinism: same inputs, same query string; a different datetime (or
/// scope date, or expiry) changes the signature.
#[test]
fn presign_is_deterministic_and_input_sensitive() {
    let a = s3_signer()
        .presign_url(PresignInput {
            host: "h.example",
            canonical_uri: "/b/k",
            expires_secs: 600,
            date: "20261111",
            datetime: "20261111T120000Z",
        })
        .unwrap();
    let b = s3_signer()
        .presign_url(PresignInput {
            host: "h.example",
            canonical_uri: "/b/k",
            expires_secs: 600,
            date: "20261111",
            datetime: "20261111T120000Z",
        })
        .unwrap();
    assert_eq!(a, b, "same inputs, same signature");
    let c = s3_signer()
        .presign_url(PresignInput {
            host: "h.example",
            canonical_uri: "/b/k",
            expires_secs: 600,
            date: "20261111",
            datetime: "20261111T120001Z",
        })
        .unwrap();
    assert_ne!(a, c, "a different datetime must change the signature");
    let d = s3_signer()
        .presign_url(PresignInput {
            host: "h.example",
            canonical_uri: "/b/k",
            expires_secs: 601,
            date: "20261111",
            datetime: "20261111T120000Z",
        })
        .unwrap();
    assert_ne!(a, d, "a different expiry must change the signature");
}

/// The S3 spec bound: expires must be 1..=604800 s (7 days). Zero,
/// negatives-by-type are impossible, but 0 and 604801 are refused honestly
/// — the shell clamps before calling, this is the engine's own gate.
#[test]
fn expiry_bounds_are_enforced() {
    let input = |e: u64| PresignInput {
        host: "h.example",
        canonical_uri: "/b/k",
        expires_secs: e,
        date: "20261111",
        datetime: "20261111T120000Z",
    };
    assert!(s3_signer().presign_url(input(0)).is_err(), "0 s refused");
    assert!(
        s3_signer().presign_url(input(604_801)).is_err(),
        ">7d refused"
    );
    assert!(s3_signer().presign_url(input(1)).is_ok(), "1 s legal");
    assert!(
        s3_signer().presign_url(input(604_800)).is_ok(),
        "exactly 7d legal"
    );
}
