//! adapter_tus wire tests (issue #10) — TUS 1.0.0 against a real tusd server.
//!
//! Three layers, mirroring adapter_s3.rs:
//! 1. Always-run unit tests pin the pure logic: offset-header parsing,
//!    Location resolution (absolute and relative).
//! 2. Env-gated wire tests: when `IUM_TUS_ENDPOINT` is unset they print
//!    `skipped` and pass; `scripts/tus-test.sh` (CI, all 3 OS) boots the
//!    pinned tusd and exports the env.
//! 3. Wire facts from the live rehearsal (captured on tusd v2.10.1 before
//!    any Rust, pinned in issue #10) are asserted, not assumed: 204 +
//!    Upload-Offset acks, 409 offset conflict, 404 bogus session, read-only
//!    HEAD verify.

use std::io::Write;

use ium_core::adapter::{AdapterError, Capability, ProgressReporter, TransferAdapter};
use ium_core::adapter_tus::{parse_upload_offset, resolve_location, TusAdapter};

use ium_core::{JobSpec, Lease};

const MIB: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// unit tests — always run
// ---------------------------------------------------------------------------

#[test]
fn parse_upload_offset_handles_real_headers() {
    assert_eq!(parse_upload_offset(Some("5242880")).unwrap(), 5242880);
    assert_eq!(parse_upload_offset(Some("0")).unwrap(), 0);
    assert!(
        parse_upload_offset(None).is_err(),
        "no Upload-Offset header is an error shape, never a silent 0"
    );
    assert!(
        parse_upload_offset(Some("")).is_err(),
        "empty header = no truth"
    );
    assert!(parse_upload_offset(Some("abc")).is_err());
    assert!(parse_upload_offset(Some("-1")).is_err());
}

#[test]
fn resolve_location_accepts_absolute_and_relative() {
    // tusd v2.10.1 returns absolute Locations (rehearsed live); the spec
    // allows relative ones — both must resolve to the same upload URL.
    assert_eq!(
        resolve_location(
            "http://127.0.0.1:8090/files/",
            "http://127.0.0.1:8090/files/abc"
        ),
        "http://127.0.0.1:8090/files/abc"
    );
    assert_eq!(
        resolve_location("http://127.0.0.1:8090/files/", "/files/abc"),
        "http://127.0.0.1:8090/files/abc"
    );
    assert_eq!(
        resolve_location("http://127.0.0.1:8090/files/", "files/abc"),
        "http://127.0.0.1:8090/files/files/abc",
        "path-relative reference merges onto the base directory (RFC 3986 §5.2)"
    );
}

#[test]
fn tus_adapter_declares_resumable_capability() {
    let a = TusAdapter::new();
    assert_eq!(a.name(), "tus");
    assert!(a.capabilities().has(Capability::RESUMABLE));
}

// ---------------------------------------------------------------------------
// wire tests — env-gated, skipped without a server
// ---------------------------------------------------------------------------

fn live() -> Option<(TusAdapter, String)> {
    let endpoint = match std::env::var("IUM_TUS_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipped: IUM_TUS_ENDPOINT not set");
            return None;
        }
    };
    Some((TusAdapter::new(), endpoint))
}

fn spec(endpoint: &str, path: &std::path::Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "tus".into(),
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
    let path = dir.path().join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    let mut seed = 0x1Au8;
    let mut left = size;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let n = buf.len().min(left as usize);
        for b in buf[..n].iter_mut() {
            seed = seed.wrapping_mul(31).wrapping_add(7);
            *b = seed;
        }
        f.write_all(&buf[..n]).unwrap();
        left -= n as u64;
    }
    f.sync_all().unwrap();
    path
}

/// The read-back GET: plain TUS 1.0.0 has no download; tusd's GET is its own
/// extension, so this helper lives in the TEST, not the adapter.
async fn download(_endpoint: &str, session: &str) -> Vec<u8> {
    let resp = reqwest::get(session.to_string())
        .await
        .unwrap_or_else(|e| panic!("GET download from {session}: {e}"));
    assert_eq!(resp.status().as_u16(), 200, "download from {session}");
    resp.bytes().await.unwrap().to_vec()
}

#[tokio::test]
async fn tus_probe_declares_resumable_session() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "probe.bin", 12 * MIB);
    let s = spec(&endpoint, &path, 12 * MIB);

    let probe = adapter.probe(&s).await.unwrap();
    assert!(probe.resumable, "TUS is resumable by definition");
    assert_eq!(
        probe.max_chunk_bytes, None,
        "TUS declares no per-call ceiling — the 409-realign path is the honesty mechanism"
    );
    let loc = probe.session_token.expect("Location is the session token");
    assert!(loc.starts_with("http"), "absolute Location on tusd: {loc}");
}

#[tokio::test]
async fn tus_full_lifecycle_two_parts_and_readback() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let size: u64 = 12 * MIB; // 5 MiB + 7 MiB
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "life.bin", size);
    let src = std::fs::read(&path).unwrap();
    let s = spec(&endpoint, &path, size);

    let probe = adapter.probe(&s).await.unwrap();
    let loc = probe.session_token.unwrap();

    let p1 = ProgressReporter::new();
    let c1 = adapter
        .upload_chunk(&s, &lease(0, 0, 5 * MIB), &p1)
        .await
        .unwrap();
    assert_eq!(c1.bytes_confirmed, 5 * MIB, "server acked the whole part");
    assert_eq!(c1.adapter_token.as_deref(), Some("5242880"));
    assert_eq!(
        p1.moved(),
        5 * MIB,
        "telemetry counts what crossed the wire"
    );

    let p2 = ProgressReporter::new();
    let c2 = adapter
        .upload_chunk(&s, &lease(1, 5 * MIB, size), &p2)
        .await
        .unwrap();
    assert_eq!(c2.bytes_confirmed, 7 * MIB);

    let body = download(&endpoint, &loc).await;
    assert_eq!(
        body, src,
        "read-back bytes equal source (integrity beyond HEAD)"
    );

    let v = adapter.verify(&s).await.unwrap();
    assert!(v.ok, "offset == length == source_size verifies");
}

#[tokio::test]
async fn tus_partial_confirm_counts_only_acked_delta() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let size: u64 = 8 * MIB;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "partial.bin", size);
    let s = spec(&endpoint, &path, size);

    let probe = adapter.probe(&s).await.unwrap();
    let loc = probe.session_token.unwrap();

    // The LEASE says resume from 1 MiB in — so the server must genuinely hold
    // 1 MiB first (the journal claims it because a previous call acked it).
    let src = std::fs::read(&path).unwrap();
    let client = reqwest::Client::new();
    let st = client
        .patch(&loc)
        .header("Upload-Offset", "0")
        .header("Tus-Resumable", "1.0.0")
        .header("Content-Type", "application/offset+octet-stream")
        .body(src[..MIB as usize].to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(
        st.status().as_u16(),
        204,
        "seed the server to 1 MiB (journal truth)"
    );

    // Then the call sends [1 MiB, 5 MiB) and must confirm exactly 4 MiB.
    let mut l = lease(0, 0, 5 * MIB);
    l.resume_from = MIB;
    let p = ProgressReporter::new();
    let c = adapter.upload_chunk(&s, &l, &p).await.unwrap();
    assert_eq!(
        c.bytes_confirmed,
        4 * MIB,
        "acked delta = new offset - resume_from"
    );
    assert_eq!(c.adapter_token.as_deref(), Some("5242880"));
    assert_eq!(p.moved(), 4 * MIB);
}

#[tokio::test]
async fn tus_offset_conflict_realigns_from_server_offset() {
    // The crash-recovery dance: a killed in-flight PATCH left the server
    // AHEAD of the journal. The adapter must absorb the drift (409 → HEAD →
    // re-PATCH from server offset) and report the honest by-this-call total.
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let size: u64 = 8 * MIB;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "realign.bin", size);
    let s = spec(&endpoint, &path, size);

    let probe = adapter.probe(&s).await.unwrap();
    let loc = probe.session_token.unwrap();

    // Simulate the lost-confirm window: 3 MiB reach the server (one PATCH
    // from 0), the journal still says 1 MiB (resume_from) — the killed call
    // acked 2 MiB that were never journaled.
    let src = std::fs::read(&path).unwrap();
    let client = reqwest::Client::new();
    let st = client
        .patch(&loc)
        .header("Upload-Offset", "0")
        .header("Tus-Resumable", "1.0.0")
        .header("Content-Type", "application/offset+octet-stream")
        .body(src[..(3 * MIB) as usize].to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(
        st.status().as_u16(),
        204,
        "drift PATCH acked: server at 3 MiB"
    );

    // Now the adapter call with the STALE journaled offset (1 MiB):
    // a naive PATCH would 409; the adapter realigns and finishes the chunk.
    let mut l = lease(0, 0, 5 * MIB);
    l.resume_from = MIB; // journal truth, 2 MiB behind the server
    let p = ProgressReporter::new();
    let c = adapter.upload_chunk(&s, &l, &p).await.unwrap();
    assert_eq!(
        c.bytes_confirmed,
        5 * MIB - MIB,
        "confirm = final offset - resume_from: 2 MiB absorbed drift + 2 MiB newly sent"
    );
    assert_eq!(c.adapter_token.as_deref(), Some("5242880"));
    // Telemetry floor, not exactness: the SUCCESSFUL re-PATCH must push the
    // [server_offset, end) tail (2 MiB). The failed 409 attempt may buffer a
    // few KiB into the socket before the rejection lands (observed: 24576 B),
    // so this call's telemetry is tail + ε — the exact-bytes claim belongs to
    // calls with no failed attempt (see the restart drill).
    assert!(
        p.moved() >= 2 * MIB,
        "at least the missing tail crossed the wire, got {}",
        p.moved()
    );

    let v = adapter.verify(&s).await.unwrap();
    assert!(!v.ok, "5 of 8 MiB present — not done yet");
}

#[tokio::test]
async fn tus_restart_drill_zero_reupload_of_received_bytes() {
    // The #9 restart drill, TUS edition (in-process): a NEW adapter instance
    // reattaches via the journaled probe event and only the bytes the server
    // has NOT received cross the wire — byte-level, not part-level.
    let Some((_adapter, endpoint)) = live() else {
        return;
    };
    let size: u64 = 12 * MIB;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "restart.bin", size);
    let s = spec(&endpoint, &path, size);

    let first = TusAdapter::new();
    let probe = first.probe(&s).await.unwrap();
    let journaled_loc = probe.session_token.unwrap();
    first
        .upload_chunk(&s, &lease(0, 0, 5 * MIB), &ProgressReporter::new())
        .await
        .unwrap();
    drop(first); // ---- the "kill -9": process state gone ----

    let second = TusAdapter::new();
    assert!(
        second.session_for(&s.endpoint).is_none(),
        "fresh adapter holds no session"
    );
    second.restore_session(&s.endpoint, journaled_loc); // host replays the probe event

    let p2 = ProgressReporter::new();
    let c2 = second
        .upload_chunk(&s, &lease(1, 5 * MIB, size), &p2)
        .await
        .unwrap();
    assert_eq!(c2.bytes_confirmed, size - 5 * MIB);
    assert_eq!(
        p2.moved(),
        size - 5 * MIB,
        "only the missing bytes moved: what the server already received was NOT re-uploaded"
    );
    let v = second.verify(&s).await.unwrap();
    assert!(v.ok);
}

#[tokio::test]
async fn tus_verify_refuses_to_lie_when_a_part_is_missing() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let size: u64 = 12 * MIB;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "honest.bin", size);
    let s = spec(&endpoint, &path, size);

    let probe = adapter.probe(&s).await.unwrap();
    let _loc = probe.session_token.unwrap();
    adapter
        .upload_chunk(&s, &lease(0, 0, 5 * MIB), &ProgressReporter::new())
        .await
        .unwrap();

    let v1 = adapter.verify(&s).await.unwrap();
    assert!(!v1.ok, "5 MiB of 12 MiB must NOT verify");
    // Read-only verify: the upload must remain incomplete (HEAD says 5 MiB).
    let v2 = adapter.verify(&s).await.unwrap();
    assert!(!v2.ok, "and nothing was completed behind the caller's back");
}

#[tokio::test]
async fn tus_bogus_session_maps_to_session_lost() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "bogus.bin", 6 * MIB);
    let s = spec(&endpoint, &path, 6 * MIB);

    adapter.restore_session(&s.endpoint, format!("{endpoint}does-not-exist"));
    let err = adapter
        .upload_chunk(&s, &lease(0, 0, 6 * MIB), &ProgressReporter::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, AdapterError::SessionLost(_)),
        "404 on an unknown upload → SessionLost: {err}"
    );
}

#[tokio::test]
async fn tus_zero_byte_upload_verifies_without_a_patch() {
    // Issue #5 invariant shape: even a 0-byte upload gets its server
    // round-trips (POST creation + HEAD verify) — the honest badge is the
    // empty PUT's twin. Pinned live against tusd.
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "empty.bin", 0);
    let s = spec(&endpoint, &path, 0);

    let probe = adapter.probe(&s).await.unwrap();
    assert!(probe.resumable);
    assert!(
        probe.session_token.is_some(),
        "creation happened even for 0 bytes"
    );

    // The manager leases the [(0, 0)] chunk (issue #5): the adapter turns the
    // empty span into a HEAD-based confirm — a real server round-trip, zero
    // pointless empty PATCHes.
    let c = adapter
        .upload_chunk(&s, &lease(0, 0, 0), &ProgressReporter::new())
        .await
        .unwrap();
    assert_eq!(
        c.bytes_confirmed, 0,
        "the span is 0 — confirmed bytes are 0"
    );

    let v = adapter.verify(&s).await.unwrap();
    assert!(v.ok, "0 == 0 verifies");
}
