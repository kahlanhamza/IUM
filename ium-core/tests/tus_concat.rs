//! tus concatenation lane wire tests (issue #50, M8 rung 3).
//!
//! Layers, mirroring adapter_tus.rs:
//! 1. Always-run unit tests pin the PURE logic: the byte-range partition
//!    (partial *i* owns `[i·L, min((i+1)·L, total))`) and the session-token
//!    encoding (classic URL vs JSON concat map — the journaled shape).
//! 2. Raw-HTTP wire legs pin the SERVER vectors captured live on tusd
//!    v2.10.1 (`scripts/issue50_wire_rehearsal.sh`, facts 1–8 on the issue):
//!    OPTIONS advertises `concatenation`; partial creation + concurrent
//!    PATCHes; eager final materialization; byte-exact GET. These are green
//!    at ANY HEAD — they test the server, not the adapter.
//! 3. Adapter legs drive the concatenation LANE through the real adapter
//!    API (`probe_fleet` / concat `upload_chunk` / `finalize`) — the RED at
//!    HEAD is a compile failure (no `Upload-Concat` anywhere yet), the
//!    loop's API-contract precedent (`verify_is_required`).
//!
//! Env-gated like the rest of the suite: `scripts/tus-test.sh --concat`
//! boots the pinned server and exports `IUM_TUS_ENDPOINT`. The @tus/server
//! lane has NO concatenation extension (the delta is recorded on the
//! issue) — server-vector and adapter legs skip LOUD there, and the
//! fallback leg runs ONLY there.

use std::io::Write;

use ium_core::adapter::ProgressReporter;
use ium_core::adapter::TransferAdapter;
use ium_core::adapter_tus::{
    concat_partition, encode_concat_token, parse_session_token, SessionToken, TusAdapter,
};

use ium_core::{JobSpec, Lease};

const KIB: u64 = 1024;
const PIECE: u64 = 256 * KIB; // 3 partials × 256 KiB = 768 KiB — fast on CI

// ---------------------------------------------------------------------------
// helpers
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

fn lane_server() -> String {
    std::env::var("IUM_TUS_SERVER").unwrap_or_else(|_| "tusd".into())
}

/// Loud per-test skip: the concatenation vectors are tusd-only —
/// @tus/server 2.4.7 does not implement the extension (recorded delta).
fn requires_concat() -> bool {
    if lane_server() == "tusd" {
        return true;
    }
    eprintln!(
        "skipped (loud, #50): concatenation needs tusd; {} lane = @tus/server \
         (no Tus-Extension: concatenation — the fallback leg runs there instead)",
        lane_server()
    );
    false
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
    let mut seed = 0x5Au8;
    let mut left = size;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let n = buf.len().min(left as usize);
        for b in buf[..n].iter_mut() {
            seed = seed.wrapping_mul(33).wrapping_add(11);
            *b = seed;
        }
        f.write_all(&buf[..n]).unwrap();
        left -= n as u64;
    }
    f.sync_all().unwrap();
    path
}

/// Distinct per-partial payloads so the read-back proves ORDER, not just size.
fn piece_bytes(i: usize) -> Vec<u8> {
    vec![0xA0 + i as u8; PIECE as usize]
}

/// The read-back GET (tusd's own download extension, as in adapter_tus.rs).
async fn download(session: &str) -> Vec<u8> {
    let resp = reqwest::get(session.to_string())
        .await
        .unwrap_or_else(|e| panic!("GET download from {session}: {e}"));
    assert_eq!(resp.status().as_u16(), 200, "download from {session}");
    resp.bytes().await.unwrap().to_vec()
}

// ---------------------------------------------------------------------------
// unit tests — always run (pure logic)
// ---------------------------------------------------------------------------

#[test]
fn concat_partition_covers_every_byte_exactly_once() {
    // The happy shape: 3 × 256 KiB.
    let p = concat_partition(3 * PIECE, 3);
    assert_eq!(
        p,
        vec![(0, PIECE), (PIECE, 2 * PIECE), (2 * PIECE, 3 * PIECE)]
    );
    // The remainder shape: 10 bytes over 3 sessions → ceil(10/3) = 4.
    let p = concat_partition(10, 3);
    assert_eq!(p, vec![(0, 4), (4, 8), (8, 10)]);
    // A partition must ALWAYS tile the total with no gaps and no overlaps.
    for &(total, n) in &[
        (0u64, 3u32),
        (1, 4),
        (7, 3),
        (64 * KIB + 1, 3),
        (1 << 20, 5),
    ] {
        let p = concat_partition(total, n);
        assert_eq!(p.first().unwrap().0, 0);
        assert_eq!(p.last().unwrap().1, total, "total={total} n={n}");
        for w in p.windows(2) {
            assert_eq!(w[0].1, w[1].0, "gap/overlap at total={total} n={n}");
        }
    }
    // n clamps to at least 1 (a 0 session count is nonsense, fleet_for rule).
    assert_eq!(concat_partition(PIECE, 0), vec![(0, PIECE)]);
}

#[test]
fn session_token_round_trips_classic_and_concat() {
    // Classic journals keep a plain URL — dispatch must not mangle it.
    let classic = "http://127.0.0.1:8090/files/abc";
    assert_eq!(
        parse_session_token(classic),
        SessionToken::Classic(classic.into())
    );
    // The concat shape round-trips partials and the final URL.
    let raw = encode_concat_token(
        &["http://x/files/p0".into(), "http://x/files/p1".into()],
        Some("http://x/files/final"),
    );
    assert!(
        raw.starts_with('{'),
        "concat token is the JSON shape: {raw}"
    );
    match parse_session_token(&raw) {
        SessionToken::Concat {
            partials,
            final_url,
        } => {
            assert_eq!(partials, vec!["http://x/files/p0", "http://x/files/p1"]);
            assert_eq!(final_url.as_deref(), Some("http://x/files/final"));
        }
        other => panic!("expected concat token, got {other:?}"),
    }
    // Pre-finalize tokens (no final URL yet) parse too — the resume shape.
    let raw = encode_concat_token(&["http://x/files/p0".into()], None);
    match parse_session_token(&raw) {
        SessionToken::Concat { final_url, .. } => assert!(final_url.is_none()),
        other => panic!("expected concat token, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// raw-HTTP wire legs — the SERVER vectors (green at any HEAD)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wire_options_advertises_concatenation() {
    let Some((_adapter, endpoint)) = live() else {
        return;
    };
    let resp = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &endpoint)
        .header("Tus-Resumable", "1.0.0")
        .send()
        .await
        .unwrap();
    let ext = resp
        .headers()
        .get("tus-extension")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if lane_server() == "tusd" {
        assert!(
            ext.split(',').any(|e| e.trim() == "concatenation"),
            "tusd 2.10.1 advertises concatenation (issue #50 fact 1), got: {ext}"
        );
    } else {
        assert!(
            !ext.split(',').any(|e| e.trim() == "concatenation"),
            "@tus/server has NO concatenation (the recorded delta), got: {ext}"
        );
    }
}

#[tokio::test]
async fn wire_concat_partial_final_readback_byte_exact() {
    let Some((_adapter, endpoint)) = live() else {
        return;
    };
    if !requires_concat() {
        return;
    }
    let client = reqwest::Client::new();
    let tr = ("Tus-Resumable", "1.0.0");

    // 2. partial creation ×3 (fact 2)
    let mut partials: Vec<String> = Vec::new();
    for _ in 0..3 {
        let r = client
            .post(&endpoint)
            .header(tr.0, tr.1)
            .header("Upload-Concat", "partial")
            .header("Upload-Length", PIECE.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 201, "partial creation (fact 2)");
        partials.push(
            r.headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
        );
    }

    // 3. concurrent PATCHes (fact 3) — three sessions advanced in parallel.
    let handles: Vec<_> = partials
        .iter()
        .enumerate()
        .map(|(i, loc)| {
            let client = client.clone();
            let loc = loc.clone();
            let body = piece_bytes(i);
            tokio::spawn(async move {
                let r = client
                    .patch(&loc)
                    .header(tr.0, tr.1)
                    .header("Upload-Offset", "0")
                    .header("Content-Type", "application/offset+octet-stream")
                    .body(body)
                    .send()
                    .await
                    .unwrap();
                (
                    r.status().as_u16(),
                    r.headers()
                        .get("upload-offset")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from),
                )
            })
        })
        .collect();
    for (i, h) in handles.into_iter().enumerate() {
        let (status, offset) = h.await.unwrap();
        assert_eq!(status, 204, "concurrent PATCH partial #{i} (fact 3)");
        assert_eq!(offset.as_deref(), Some("262144"));
    }

    // 5b. final WITHOUT Upload-Length — the server derives (fact 5b)
    let r = client
        .post(&endpoint)
        .header(tr.0, tr.1)
        .header("Upload-Concat", format!("final;{}", partials.join(" ")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 201, "final creation (fact 5b)");
    let final_url = r.headers().get("location").unwrap().to_str().unwrap();

    // 6. eager materialization (fact 6)
    let h = client
        .head(final_url)
        .header(tr.0, tr.1)
        .send()
        .await
        .unwrap();
    assert_eq!(
        h.headers()
            .get("upload-offset")
            .and_then(|v| v.to_str().ok()),
        Some("786432"),
        "HEAD final immediately == sum (fact 6: eager)"
    );

    // 7. byte-exact read-back in PARTIAL ORDER (fact 7)
    let mut expect = Vec::new();
    for i in 0..3 {
        expect.extend_from_slice(&piece_bytes(i));
    }
    assert_eq!(
        download(final_url).await,
        expect,
        "byte-exact final (fact 7)"
    );
}

// ---------------------------------------------------------------------------
// adapter legs — the LANE through the real API (RED at HEAD: compile)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adapter_probe_fleet_declares_concat_parallelism() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    if !requires_concat() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "probe.bin", 3 * PIECE);
    let s = spec(&endpoint, &path, 3 * PIECE);

    let probe = adapter.probe_fleet(&s, 3).await.unwrap();
    assert!(
        probe.concatenation,
        "tusd advertises the extension (fact 1)"
    );
    assert_eq!(
        probe.max_parallel,
        Some(3),
        "the adapter declares N = the host's seed (ADR-011 channel)"
    );
    let token = probe.session_token.expect("concat token is the session");
    match parse_session_token(&token) {
        SessionToken::Concat {
            partials,
            final_url,
        } => {
            assert_eq!(partials.len(), 3, "N partial sessions created");
            assert!(final_url.is_none(), "no final before any byte moved");
        }
        other => panic!("expected a concat session token, got {other:?}"),
    }
}

#[tokio::test]
async fn adapter_concat_parallel_parts_finalize_verify_readback() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    if !requires_concat() {
        return;
    }
    let size = 3 * PIECE;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "lane.bin", size);
    let s = spec(&endpoint, &path, size);

    let probe = adapter.probe_fleet(&s, 3).await.unwrap();
    let token = probe.session_token.clone().unwrap();

    // Three workers, three partials, CONCURRENTLY (the lane's whole point).
    let l0 = lease(0, 0, PIECE);
    let l1 = lease(1, PIECE, 2 * PIECE);
    let l2 = lease(2, 2 * PIECE, size);
    let (p0, p1, p2) = (
        ProgressReporter::new(),
        ProgressReporter::new(),
        ProgressReporter::new(),
    );
    let (c0, c1, c2) = tokio::join!(
        adapter.upload_chunk(&s, &l0, &p0),
        adapter.upload_chunk(&s, &l1, &p1),
        adapter.upload_chunk(&s, &l2, &p2),
    );
    assert_eq!(c0.unwrap().bytes_confirmed, PIECE);
    assert_eq!(c1.unwrap().bytes_confirmed, PIECE);
    assert_eq!(c2.unwrap().bytes_confirmed, PIECE);

    // Verify BEFORE finalize must refuse to lie: the server holds the bytes
    // but no materialized final exists (fact 8b — the final is the resource).
    let v0 = adapter.verify(&s).await.unwrap();
    assert!(!v0.ok, "concat not finalized yet — verify says so honestly");

    // Finalize at the all-Confirmed boundary (the manager's Finishing call).
    let final_url = adapter
        .finalize(&s)
        .await
        .unwrap()
        .expect("tus concat finalizes");
    assert!(final_url.starts_with("http"), "final Location: {final_url}");

    // Idempotent: a second finalize reuses the journaled URL (never re-POSTs
    // — fact 5b: a re-POST would create a SECOND final).
    let again = adapter.finalize(&s).await.unwrap().unwrap();
    assert_eq!(again, final_url, "finalize is idempotent per session");

    let v = adapter.verify(&s).await.unwrap();
    assert!(v.ok, "HEAD final == length == source_size verifies");

    // Byte-exact read-back, partial order preserved — the adapter uploaded
    // the SOURCE FILE, so the final must equal it byte for byte.
    let src = std::fs::read(&path).unwrap();
    assert_eq!(download(&final_url).await, src);

    // The journaled token now carries the final URL (resume re-derives).
    let stored = adapter.session_for(&s.endpoint).unwrap();
    match parse_session_token(&stored) {
        SessionToken::Concat { final_url: f, .. } => {
            assert_eq!(f.as_deref(), Some(final_url.as_str()));
        }
        other => panic!("expected concat token, got {other:?}"),
    }
    let _ = token; // journaled probe token asserted in the probe leg
}

#[tokio::test]
async fn adapter_concat_chunk_spanning_two_partials_confirms_once() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    if !requires_concat() {
        return;
    }
    let size = 3 * PIECE;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "span.bin", size);
    let s = spec(&endpoint, &path, size);

    adapter.probe_fleet(&s, 3).await.unwrap();

    // [128 KiB, 384 KiB) straddles the partial-0/partial-1 boundary: TWO
    // segments, two PATCH targets, ONE honest confirm of 256 KiB. The wire
    // pushes MORE than the lease: partial 0 is fresh (the server sits at 0,
    // below the segment's local start), so its PATCH streams from the
    // server's offset up to the segment end — the neighbor's span crosses
    // once here and is confirmed by the NEIGHBOR's journal entry later
    // (confirm-before-send). Telemetry counts what crossed: 256 KiB + 128 KiB.
    let p = ProgressReporter::new();
    let c = adapter
        .upload_chunk(&s, &lease(0, 128 * KIB, 384 * KIB), &p)
        .await
        .unwrap();
    assert_eq!(c.bytes_confirmed, 256 * KIB, "both segments confirmed");
    assert_eq!(
        p.moved(),
        384 * KIB,
        "telemetry counts exactly what crossed the wire (the fresh partial's \
         below-segment bytes included)"
    );
    assert_eq!(
        p.wasted(),
        0,
        "every pushed byte was server-persisted — nothing flushed nowhere"
    );
    // Read back BOTH partials and check the straddle landed on the right
    // sides of the boundary.
    let token = parse_session_token(&adapter.session_for(&s.endpoint).unwrap());
    let SessionToken::Concat { partials, .. } = token else {
        panic!("concat session expected");
    };
    let src = std::fs::read(&path).unwrap();
    assert_eq!(download(&partials[0]).await, src[..PIECE as usize].to_vec());
    assert_eq!(
        download(&partials[1]).await,
        src[PIECE as usize..384 * KIB as usize].to_vec(),
        "partial 1 holds exactly the pushed half (128 KiB)"
    );
}

#[tokio::test]
async fn adapter_concat_resume_per_session_zero_reupload() {
    let Some((_adapter, endpoint)) = live() else {
        return;
    };
    if !requires_concat() {
        return;
    }
    let size = 3 * PIECE;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "resume.bin", size);
    let s = spec(&endpoint, &path, size);

    let first = TusAdapter::new();
    first.probe_fleet(&s, 3).await.unwrap();
    // Partial 0 full, partial 1 half (the straddling-segment rule puts
    // [PIECE, PIECE + 128 KiB) on partial 1).
    first
        .upload_chunk(&s, &lease(0, 0, PIECE), &ProgressReporter::new())
        .await
        .unwrap();
    first
        .upload_chunk(
            &s,
            &lease(1, PIECE, PIECE + 128 * KIB),
            &ProgressReporter::new(),
        )
        .await
        .unwrap();
    let journaled = first.session_for(&s.endpoint).unwrap();
    drop(first); // ---- the kill -9 ----

    let second = TusAdapter::new();
    second.reattach(&s.endpoint, &journaled).await.unwrap();
    let p0 = ProgressReporter::new();
    let c = second
        .upload_chunk(&s, &lease(1, PIECE + 128 * KIB, 2 * PIECE), &p0)
        .await
        .unwrap();
    assert_eq!(c.bytes_confirmed, 128 * KIB);
    assert_eq!(
        p0.moved(),
        128 * KIB,
        "only the missing tail crossed the wire — per-session byte resume (#10 rules per partial)"
    );
    let p1 = ProgressReporter::new();
    second
        .upload_chunk(&s, &lease(2, 2 * PIECE, size), &p1)
        .await
        .unwrap();
    let final_url = second.finalize(&s).await.unwrap().unwrap();
    let v = second.verify(&s).await.unwrap();
    assert!(v.ok);

    let src = std::fs::read(&path).unwrap();
    assert_eq!(download(&final_url).await, src);
}

#[tokio::test]
async fn adapter_fallback_without_concat_declares_one() {
    let Some((adapter, endpoint)) = live() else {
        return;
    };
    if lane_server() != "tus-node" {
        eprintln!("skipped: the fallback leg is @tus/server-only (tusd HAS the extension)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "fallback.bin", 3 * PIECE);
    let s = spec(&endpoint, &path, 3 * PIECE);

    let probe = adapter.probe_fleet(&s, 4).await.unwrap();
    assert!(
        !probe.concatenation,
        "@tus/server does not advertise the extension"
    );
    assert_eq!(
        probe.max_parallel,
        Some(1),
        "the lane stays honest at 1 without the server's declaration (recorded delta)"
    );
    assert!(probe.session_token.is_some(), "classic session created");
}

#[tokio::test]
async fn manager_drives_concat_lane_to_done_end_to_end() {
    // The M8 acceptance shape: the REAL drive loop — probe_fleet (N
    // partials), the AIMD fleet within the declared ceiling, the Finishing
    // boundary's finalize, and verify on the final URL — Done, every byte
    // confirmed exactly once.
    let Some((_adapter, endpoint)) = live() else {
        return;
    };
    if !requires_concat() {
        return;
    }
    let size = 3 * PIECE;
    let dir = tempfile::tempdir().unwrap();
    let path = write_source(&dir, "e2e.bin", size);

    let journal_path = dir.path().join("tus-concat-e2e.db");
    let mut m = ium_core::JobManager::open(&journal_path).unwrap();
    m.register_adapter(std::sync::Arc::new(ium_core::TusAdapter::new()));

    let rec = m.create_job(&spec(&endpoint, &path, size), 3).unwrap();
    let done = m.run_job(&rec.id).await.unwrap();
    assert_eq!(done.state, ium_core::state::JobState::Done);

    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        size,
        "every byte server-acknowledged exactly once"
    );
    // The probe event journaled the discovery + the declaration channel.
    let probe_event = snap
        .events
        .iter()
        .rev()
        .find(|(_, kind, _)| kind == "probe")
        .map(|(_, _, p)| p.clone())
        .expect("probe event journaled");
    let v: serde_json::Value = serde_json::from_str(&probe_event).unwrap();
    assert_eq!(v["concatenation"], serde_json::json!(true));
    assert_eq!(v["max_parallel"], serde_json::json!(3));
    // The finalize intent/done pair is on the record (journal-before-execute).
    assert!(
        snap.events
            .iter()
            .any(|(_, kind, _)| kind == "concat_final"),
        "concat_final journaled"
    );
}
