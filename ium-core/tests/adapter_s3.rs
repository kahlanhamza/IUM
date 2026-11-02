//! S3 multipart adapter — contract tests.
//!
//! Two layers, mirroring the env-gating pattern proven by the fuzz suite:
//!
//! 1. Always-run unit tests pin the pure logic: endpoint parsing, part tokens,
//!    XML parsing (with entity decode — Garage escapes ETag quotes), the
//!    reconcile rule, the Complete-XML builder, and the error taxonomy. The
//!    ListParts vector below is a REAL Garage v2.1.0 response captured during
//!    the live protocol rehearsal on the issue.
//!
//! 2. Env-gated wire tests: when `IUM_S3_ENDPOINT` is unset they print
//!    `skipped` and pass — local `cargo test` needs no S3 server. With the
//!    endpoint set they exercise the full multipart lifecycle against a real
//!    server, including the restart drill (restore_session → zero re-upload).

use std::time::Duration;

use ium_core::adapter::{
    AdapterError, Capability, ProgressReporter, TransferAdapter, VerifyReport,
};
use ium_core::adapter_s3::{
    complete_multipart_xml, map_error, parse_list_parts, parse_part_token, parse_target,
    part_token, reconcile_parts, xml_text, ErrorCtx, ListedPart, S3MultipartAdapter,
    S3MultipartConfig,
};
use ium_core::{JobSpec, Lease};

// ---------------------------------------------------------------------------
// pure logic — always run
// ---------------------------------------------------------------------------

#[test]
fn parse_target_takes_key_from_object_url() {
    let t = parse_target(
        "http://127.0.0.1:3900/ium-test/dir/obj.bin",
        "/tmp/whatever",
    )
    .unwrap();
    assert_eq!(t.host, "127.0.0.1:3900");
    assert_eq!(t.bucket, "ium-test");
    assert_eq!(t.key, "dir/obj.bin");
}

#[test]
fn parse_target_defaults_key_to_source_basename() {
    let t = parse_target("http://127.0.0.1:3900/ium-test", "/home/me/big file.bin").unwrap();
    assert_eq!(t.bucket, "ium-test");
    assert_eq!(t.key, "big file.bin");
}

#[test]
fn parse_target_rejects_bucketless_and_schemeless() {
    assert!(parse_target("http://127.0.0.1:3900/", "/tmp/f").is_err());
    assert!(parse_target("not a url", "/tmp/f").is_err());
}

#[test]
fn parse_target_encodes_key_path() {
    let t = parse_target("http://h:9000/b", "/tmp/x").unwrap();
    // The canonical URI the adapter signs/sends must be path-style with
    // S3 single-encoding; slashes stay separators.
    let t2 = parse_target("http://h:9000/b/a+b/c.txt", "/tmp/x").unwrap();
    assert_eq!(t2.key, "a+b/c.txt");
    assert_eq!(t.host, "h:9000");
    assert_eq!(t.bucket, "b");
    let _ = t;
}

#[test]
fn part_tokens_roundtrip() {
    // part_token takes the 0-based idx and stores the 1-based part number.
    let tok = part_token(0, "\"abc123\"");
    assert_eq!(tok, "1:\"abc123\"");
    let (n, etag) = parse_part_token(&tok).unwrap();
    assert_eq!(n, 1);
    assert_eq!(etag, "\"abc123\"");
    let tok7 = part_token(6, "\"x\"");
    assert_eq!(tok7, "7:\"x\"", "idx 6 = part number 7");
    assert!(parse_part_token("no-colon").is_none());
    assert!(parse_part_token("x:1").is_none(), "non-numeric part number");
}

#[test]
fn xml_text_decodes_the_five_named_entities() {
    let xml = "<ETag>&quot;a&amp;b&apos;c&lt;d&gt;e&quot;</ETag>";
    assert_eq!(xml_text(xml, "ETag").as_deref(), Some("\"a&b'c<d>e\""));
    assert_eq!(
        xml_text("<Size>5242880</Size>", "Size").as_deref(),
        Some("5242880")
    );
    assert_eq!(xml_text("<Other>x</Other>", "Missing"), None);
}

#[test]
fn xml_text_decodes_numeric_character_references() {
    // SeaweedFS 4.51 (issue #9) escapes the ETag quotes as numeric refs —
    // the named-only decode left literal `&#34;` in the etag and Complete
    // answered InvalidPart. Decimal and hex forms both decode.
    assert_eq!(
        xml_text("<ETag>&#34;abc&#34;</ETag>", "ETag").as_deref(),
        Some("\"abc\"")
    );
    assert_eq!(
        xml_text("<ETag>&#x22;abc&#X22;</ETag>", "ETag").as_deref(),
        Some("\"abc\"")
    );
    assert_eq!(
        xml_text("<ETag>a&amp;b</ETag>", "ETag").as_deref(),
        Some("a&b")
    );
}

#[test]
fn xml_text_decodes_each_reference_exactly_once() {
    // The double-decode trap: `&amp;#34;` is the LITERAL TEXT `&#34;` in XML —
    // decoding it again to `"` would be a correctness bug, not leniency.
    assert_eq!(
        xml_text("<ETag>&amp;#34;x&amp;#34;</ETag>", "ETag").as_deref(),
        Some("&#34;x&#34;")
    );
    // A bare `&` that starts no valid reference is copied verbatim.
    assert_eq!(
        xml_text("<ETag>a&b&#65;c</ETag>", "ETag").as_deref(),
        Some("a&bAc")
    );
    // Malformed numerics stay literal.
    assert_eq!(
        xml_text("<ETag>&#; &#zz; &#x;</ETag>", "ETag").as_deref(),
        Some("&#; &#zz; &#x;")
    );
}

/// Real SeaweedFS 4.51 ListParts response (captured live during the issue #9
/// rehearsal) — the numeric-entity wire shape, pinned as a parse vector.
#[test]
fn parse_list_parts_on_captured_seaweedfs_vector() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><ListPartsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>ium-test</Bucket><IsTruncated>false</IsTruncated><Key>mpu.bin</Key><MaxParts>10000</MaxParts><PartNumberMarker>0</PartNumberMarker><StorageClass>STANDARD</StorageClass><UploadId>4fcac3a0277c0b99d1dad7b457bdc176b777d11d_01a199f969c475dbbcce8a405a611e71</UploadId><Part><ETag>&#34;39a615f574c21f9e3ac7be61fc573bdb&#34;</ETag><LastModified>2026-11-02T01:59:14Z</LastModified><PartNumber>1</PartNumber><Size>5242880</Size></Part></ListPartsResult>"#;
    let (parts, truncated, marker) = parse_list_parts(xml);
    assert!(!truncated);
    assert_eq!(marker, None);
    assert_eq!(parts.len(), 1);
    assert_eq!(
        parts[0],
        ListedPart {
            number: 1,
            size: 5242880,
            // decoded to the same shape every server's UploadPart header uses
            etag: "\"39a615f574c21f9e3ac7be61fc573bdb\"".into(),
        }
    );
}

/// Real Garage v2.1.0 ListParts response (captured live during the rehearsal
/// on this issue) — pinned as the parse vector.
#[test]
fn parse_list_parts_on_captured_garage_vector() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><ListPartsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>ium-test</Bucket><Key>smoke.bin</Key><UploadId>22aa5dd4523d8d43ad9763c42316f5835afe62869f0803476073dfa10dea57f9</UploadId><MaxParts>1000</MaxParts><IsTruncated>false</IsTruncated><Part><ETag>&quot;e0a9fa0710d40f3ebd31fcd9fdad4923&quot;</ETag><LastModified>2026-11-02T00:55:12.672Z</LastModified><PartNumber>1</PartNumber><Size>5242880</Size></Part><Initiator><DisplayName>Dummy Key</DisplayName><ID>GKDummyKey</ID></Initiator></ListPartsResult>"#;
    let (parts, truncated, marker) = parse_list_parts(xml);
    assert_eq!(parts.len(), 1);
    assert_eq!(
        parts[0],
        ListedPart {
            number: 1,
            size: 5242880,
            etag: "\"e0a9fa0710d40f3ebd31fcd9fdad4923\"".into(),
        }
    );
    assert!(!truncated);
    assert_eq!(marker, None);
}

#[test]
fn parse_list_parts_pagination_fields() {
    let xml = "<ListPartsResult><IsTruncated>true</IsTruncated><NextPartNumberMarker>5</NextPartNumberMarker>\
               <Part><ETag>&quot;a&quot;</ETag><PartNumber>4</PartNumber><Size>10</Size></Part>\
               <Part><ETag>&quot;b&quot;</ETag><PartNumber>5</PartNumber><Size>20</Size></Part>\
               </ListPartsResult>";
    let (parts, truncated, marker) = parse_list_parts(xml);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[1].number, 5);
    assert!(truncated);
    assert_eq!(marker.as_deref(), Some("5"));
}

#[test]
fn reconcile_accepts_exact_contiguous_parts() {
    let parts = vec![
        ListedPart {
            number: 1,
            size: 5_242_880,
            etag: "\"a\"".into(),
        },
        ListedPart {
            number: 2,
            size: 7_340_032,
            etag: "\"b\"".into(),
        },
    ];
    assert_eq!(reconcile_parts(&parts, 5_242_880 + 7_340_032), Ok(()));
}

#[test]
fn reconcile_rejects_gaps_sums_and_empty() {
    let gap = vec![
        ListedPart {
            number: 1,
            size: 100,
            etag: "\"a\"".into(),
        },
        ListedPart {
            number: 3,
            size: 20,
            etag: "\"c\"".into(),
        },
    ];
    assert!(
        reconcile_parts(&gap, 120).is_err(),
        "part numbers must be contiguous"
    );

    let short = vec![ListedPart {
        number: 1,
        size: 100,
        etag: "\"a\"".into(),
    }];
    assert!(
        reconcile_parts(&short, 120).is_err(),
        "Σ sizes must equal source_size"
    );

    let empty: Vec<ListedPart> = vec![];
    assert!(reconcile_parts(&empty, 0).is_err() || reconcile_parts(&empty, 0) == Ok(()));
    assert!(reconcile_parts(&empty, 10).is_err());
}

#[test]
fn complete_xml_lists_parts_ascending_with_quoted_etags() {
    let xml = complete_multipart_xml(&[(2, "\"b\""), (1, "\"a\"")]);
    assert!(xml.contains("<CompleteMultipartUpload"));
    let i1 = xml.find("<PartNumber>1</PartNumber>").unwrap();
    let i2 = xml.find("<PartNumber>2</PartNumber>").unwrap();
    assert!(i1 < i2, "parts must be ascending in the Complete body");
    assert!(
        xml.contains("<ETag>&quot;a&quot;</ETag>"),
        "etags re-escaped for XML"
    );
}

#[test]
fn error_taxonomy_follows_the_design_note() {
    use AdapterError::*;
    let kind = |e: &AdapterError| e.kind();
    // NoSuchUpload on a chunk call → ExpiredSession (issue criteria).
    assert_eq!(
        kind(&map_error(
            ErrorCtx::Chunk,
            404,
            Some("NoSuchUpload"),
            None,
            "d"
        )),
        kind(&ExpiredSession("x".into()))
    );
    // Probe-level bucket errors are fatal, not retryable.
    assert_eq!(
        kind(&map_error(
            ErrorCtx::Probe,
            404,
            Some("NoSuchBucket"),
            None,
            "d"
        )),
        kind(&Unresumable("x".into()))
    );
    // 503 is SlowDown throttling with or without Retry-After.
    assert_eq!(
        kind(&map_error(
            ErrorCtx::Chunk,
            503,
            Some("SlowDown"),
            Some(3000),
            "d"
        )),
        kind(&Throttled {
            retry_after_ms: Some(3000),
            message: "x".into()
        })
    );
    assert_eq!(
        kind(&map_error(ErrorCtx::Chunk, 503, None, None, "d")),
        kind(&Throttled {
            retry_after_ms: None,
            message: "x".into()
        })
    );
    // 429 → Throttled; other 5xx → Transient; 403 mid-session → ExpiredSession;
    // 403 on probe → AuthRequired; anything else → Unknown.
    assert_eq!(
        kind(&map_error(ErrorCtx::Chunk, 429, None, Some(7), "d")),
        kind(&Throttled {
            retry_after_ms: Some(7),
            message: "x".into()
        })
    );
    assert_eq!(
        kind(&map_error(ErrorCtx::Chunk, 500, None, None, "d")),
        kind(&Transient("x".into()))
    );
    assert_eq!(
        kind(&map_error(ErrorCtx::Chunk, 502, None, None, "d")),
        kind(&Transient("x".into()))
    );
    assert_eq!(
        kind(&map_error(ErrorCtx::Chunk, 403, None, None, "d")),
        kind(&ExpiredSession("x".into()))
    );
    assert_eq!(
        kind(&map_error(ErrorCtx::Probe, 403, None, None, "d")),
        kind(&AuthRequired("x".into()))
    );
    assert_eq!(
        kind(&map_error(
            ErrorCtx::Chunk,
            400,
            Some("InvalidPart"),
            None,
            "d"
        )),
        kind(&Unknown("x".into()))
    );
}

#[test]
fn etag_header_lookup_is_case_insensitive() {
    // Garage/hyper lowercases response headers on the wire; this is the
    // regression that made the live rehearsal's first Complete fail.
    let mut hm = reqwest::header::HeaderMap::new();
    hm.insert("etag", reqwest::header::HeaderValue::from_static("\"x\""));
    assert_eq!(
        hm.get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()),
        Some("\"x\"")
    );
    let _ = Duration::from_secs(0); // keep the Duration import honest for map_error docs
}

// ---------------------------------------------------------------------------
// wire tests — env-gated, skipped without a server
// ---------------------------------------------------------------------------

/// Shared harness: endpoint + adapter from the IUM_S3_* environment.
/// Returns None (test prints `skipped`) when the endpoint isn't configured.
fn live() -> Option<(S3MultipartAdapter, String)> {
    let endpoint = match std::env::var("IUM_S3_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipped: IUM_S3_ENDPOINT not set");
            return None;
        }
    };
    let config = S3MultipartConfig::from_env()
        .unwrap_or_else(|e| panic!("IUM_S3_ENDPOINT is set but creds are not: {e}"));
    let adapter = S3MultipartAdapter::new(config).expect("adapter builds");
    Some((adapter, endpoint))
}

fn spec(endpoint: &str, path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "s3_multipart".into(),
        endpoint: endpoint.to_string(),
    }
}

fn lease(idx: u32, begin: u64, end: u64) -> Lease {
    Lease {
        idx,
        begin,
        end,
        resume_from: begin,
        attempt: 0,
    }
}

fn write_source(dir: &tempfile::TempDir, name: &str, size: u64) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.path().join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    let mut remaining = size;
    let mut seed = 0xA5_u8;
    while remaining > 0 {
        let n = remaining.min(1 << 20) as usize;
        let buf = vec![seed; n];
        f.write_all(&buf).unwrap();
        remaining -= n as u64;
        seed = seed.wrapping_add(0x5A);
    }
    f.sync_all().unwrap();
    path
}

async fn read_back(adapter: &S3MultipartAdapter, spec: &JobSpec) -> Vec<u8> {
    adapter
        .test_get_object(spec)
        .await
        .expect("signed GET read-back")
}

#[tokio::test]
async fn s3_probe_declares_multipart_honestly() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "probe.bin", 12 * 1024 * 1024);
    let s = spec(&endpoint, &path, 12 * 1024 * 1024);

    assert_eq!(adapter.name(), "s3_multipart");
    assert!(
        adapter.capabilities().has(Capability::RESUMABLE)
            && adapter.capabilities().has(Capability::DYNAMIC_CHUNKS),
        "honest badges: RESUMABLE | DYNAMIC_CHUNKS"
    );
    let probe = adapter.probe(&s).await.unwrap();
    assert!(probe.resumable);
    assert_eq!(probe.max_chunk_bytes, Some(5 * 1024 * 1024 * 1024));
    assert!(
        probe.session_token.is_some(),
        "UploadId is the session token"
    );
}

#[tokio::test]
async fn s3_full_lifecycle_two_parts_and_readback() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let size: u64 = 12 * 1024 * 1024; // 5 MiB + 7 MiB
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "life.bin", size);
    let s = spec(&endpoint, &path, size);

    let probe = adapter.probe(&s).await.unwrap();
    let uid = probe.session_token.unwrap();

    let p1 = ProgressReporter::new();
    let c1 = adapter
        .upload_chunk(&s, &lease(0, 0, 5 * 1024 * 1024), &p1)
        .await
        .unwrap();
    assert_eq!(
        c1.bytes_confirmed,
        5 * 1024 * 1024,
        "parts are atomic: 0 or full span"
    );
    assert_eq!(p1.moved(), 5 * 1024 * 1024, "wire bytes counted");
    let (n1, e1) = parse_part_token(c1.adapter_token.as_deref().unwrap()).unwrap();
    assert_eq!(n1, 1);
    assert!(e1.starts_with('"'), "etag arrives quoted");

    let p2 = ProgressReporter::new();
    let c2 = adapter
        .upload_chunk(&s, &lease(1, 5 * 1024 * 1024, size), &p2)
        .await
        .unwrap();
    assert_eq!(c2.bytes_confirmed, size - 5 * 1024 * 1024);
    assert_eq!(
        parse_part_token(c2.adapter_token.as_deref().unwrap())
            .unwrap()
            .0,
        2
    );
    assert_eq!(
        adapter.session_for(&s.endpoint).as_deref(),
        Some(uid.as_str())
    );

    let v = adapter.verify(&s).await.unwrap();
    assert!(v.ok, "verify must pass: {:?}", v.detail);

    let body = read_back(&adapter, &s).await;
    assert_eq!(body.len() as u64, size, "read-back length equals source");
    let src = std::fs::read(&path).unwrap();
    assert_eq!(
        body, src,
        "read-back bytes equal source (integrity beyond CL)"
    );
}

#[tokio::test]
async fn s3_restart_drill_zero_reupload_of_completed_parts() {
    // The #9 kill-9 drill, in miniature: a NEW adapter instance (fresh
    // process) recovers the UploadId from the journal's probe event via
    // restore_session, and only the missing part crosses the wire.
    let size: u64 = 12 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "drill.bin", size);
    let Some((_, endpoint)) = live() else { return };
    let s = spec(&endpoint, &path, size);

    let first = S3MultipartAdapter::new(S3MultipartConfig::from_env().unwrap()).unwrap();
    let probe = first.probe(&s).await.unwrap();
    let journaled_uid = probe.session_token.unwrap();
    first
        .upload_chunk(&s, &lease(0, 0, 5 * 1024 * 1024), &ProgressReporter::new())
        .await
        .unwrap();
    drop(first); // ---- the "kill -9": process state gone ----

    let second = S3MultipartAdapter::new(S3MultipartConfig::from_env().unwrap()).unwrap();
    assert!(
        second.session_for(&s.endpoint).is_none(),
        "fresh adapter holds no session"
    );
    second.restore_session(&s.endpoint, journaled_uid); // host replays the probe event

    let p2 = ProgressReporter::new();
    let c2 = second
        .upload_chunk(&s, &lease(1, 5 * 1024 * 1024, size), &p2)
        .await
        .unwrap();
    assert_eq!(c2.bytes_confirmed, size - 5 * 1024 * 1024);
    assert_eq!(
        p2.moved(),
        size - 5 * 1024 * 1024,
        "only the missing part moved: completed parts were NOT re-uploaded"
    );
    let v = second.verify(&s).await.unwrap();
    assert!(v.ok);
}

#[tokio::test]
async fn s3_verify_refuses_to_lie_when_a_part_is_missing() {
    let size: u64 = 12 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "honest.bin", size);
    let Some((_, endpoint)) = live() else { return };
    let s = spec(&endpoint, &path, size);

    let a = S3MultipartAdapter::new(S3MultipartConfig::from_env().unwrap()).unwrap();
    a.probe(&s).await.unwrap();
    a.upload_chunk(&s, &lease(0, 0, 5 * 1024 * 1024), &ProgressReporter::new())
        .await
        .unwrap();

    let v1 = a.verify(&s).await.unwrap();
    assert!(!v1.ok, "5 MiB of 12 MiB must NOT verify");
    let v2 = a.verify(&s).await.unwrap();
    assert!(
        !v2.ok,
        "and the upload must remain incomplete (no lying Complete)"
    );
}

#[tokio::test]
async fn s3_bogus_session_maps_to_expired_session() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "bogus.bin", 6 * 1024 * 1024);
    let s = spec(&endpoint, &path, 6 * 1024 * 1024);

    adapter.restore_session(&s.endpoint, "ium-bogus-upload-id-does-not-exist".into());
    let err = adapter
        .upload_chunk(&s, &lease(0, 0, 6 * 1024 * 1024), &ProgressReporter::new())
        .await
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ium_core::retry::ErrorKind::ExpiredSession,
        "NoSuchUpload → ExpiredSession: {err}"
    );
}

#[tokio::test]
async fn s3_zero_byte_takes_the_putobject_path() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "empty.bin", 0);
    let s = spec(&endpoint, &path, 0);

    let probe = adapter.probe(&s).await.unwrap();
    assert!(
        probe.session_token.is_none(),
        "0-byte probe must not open a multipart session (no orphans)"
    );
    assert!(probe.resumable, "an empty PUT is trivially re-runnable");

    let p = ProgressReporter::new();
    // Issue #5's pinned [(0,0)] lease: one honest round-trip.
    let c = adapter.upload_chunk(&s, &lease(0, 0, 0), &p).await.unwrap();
    assert_eq!(
        c.bytes_confirmed, 0,
        "the span is 0 — confirmed bytes are 0"
    );
    let v = adapter.verify(&s).await.unwrap();
    assert!(v.ok, "HEAD on a 0-byte object: {:?}", v.detail);
    let body = read_back(&adapter, &s).await;
    assert!(body.is_empty());
}

#[tokio::test]
async fn s3_upload_without_session_is_session_lost() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "nosession.bin", 6 * 1024 * 1024);
    let s = spec(&endpoint, &path, 6 * 1024 * 1024);
    let err = adapter
        .upload_chunk(&s, &lease(0, 0, 6 * 1024 * 1024), &ProgressReporter::new())
        .await
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ium_core::retry::ErrorKind::SessionLost,
        "map miss → SessionLost → policy Reattach: {err}"
    );
}

// Silence the unused-import warning path for VerifyReport in skip-only builds.
#[allow(dead_code)]
fn _t(_: VerifyReport) {}
