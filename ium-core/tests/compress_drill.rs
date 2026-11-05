//! The compress-mode live drill (issue #53 — the Cause-D byte reduction,
//! measured on the loopback tus lane).
//!
//! Legs:
//! * **Compressible** (4 MiB repetitive text): managed job through
//!   `CompressAdapter(TusAdapter)` → Done; the journal's manifest carries
//!   mode/level/BOTH hashes and `archive_size < source_size` (the wire paid
//!   LESS than the source — the Cause-D point); the ARCHIVE is read back
//!   from the server, decoded (checksummed frame) and the decoded bytes are
//!   byte-exact vs the source; the staging archive is gone after Done (D6).
//! * **Hostile** (4 MiB video-like bytes): the gate DECLINES in seconds —
//!   plain job, journaled reason + ratio, zero temp files.
//!
//! Env-gated (`IUM_COMPRESS=1` + `IUM_TUS_ENDPOINT`) — the PR lane skips
//! LOUDLY.

use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

use ium_core::adapter_compress::staging_archive;
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{CompressAdapter, JobSpec, TusAdapter};

const KIB: u64 = 1024;
const SIZE: u64 = 4 * 1024 * KIB;

fn write_source(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn mk_text(len: usize) -> Vec<u8> {
    let words = [
        "the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog ",
    ];
    let mut v = Vec::with_capacity(len);
    let mut x: u32 = 7;
    while v.len() < len {
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        v.extend_from_slice(words[(x >> 16) as usize % words.len()].as_bytes());
    }
    v.truncate(len);
    v
}

fn mk_video(len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    let mut x: u32 = 0x9E37_79B9;
    for _ in 0..len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        v.push((x >> 24) as u8);
    }
    v
}

fn last_event(events: &[(i64, String, String)], kind: &str) -> Option<serde_json::Value> {
    events
        .iter()
        .rev()
        .find(|(_, k, _)| k == kind)
        .and_then(|(_, _, p)| serde_json::from_str(p).ok())
}

async fn readback(url: &str) -> Vec<u8> {
    let resp = reqwest::get(url.to_string())
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    assert_eq!(resp.status().as_u16(), 200, "read-back {url}");
    resp.bytes().await.unwrap().to_vec()
}

/// Run one managed compress job; returns (journal snapshot, wall seconds).
async fn run_job(
    endpoint: &str,
    source: &std::path::Path,
    size: u64,
) -> (ium_core::JobSnapshot, f64) {
    let dir = source.parent().unwrap().to_path_buf();
    let db = dir.join("journal.db");
    let mut m = JobManager::open(&db).expect("manager");
    m.register_adapter(Arc::new(CompressAdapter::new(Arc::new(TusAdapter::new()))));
    let spec = JobSpec {
        source_path: source.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "compress".into(),
        endpoint: endpoint.to_string(),
    };
    let rec = m.create_job(&spec, 4).expect("job");
    let started = Instant::now();
    let done = tokio::time::timeout(duration_secs(240), m.run_job(&rec.id))
        .await
        .expect("leg within budget")
        .expect("run ok");
    let wall = started.elapsed().as_secs_f64();
    if done.state != JobState::Done {
        let snap = m.snapshot(&rec.id).unwrap();
        for (ts, k, payload) in snap.events.iter().rev().take(8) {
            println!("EVENT [{ts}] {k}: {payload}");
        }
    }
    assert_eq!(done.state, JobState::Done, "the leg converges Done");
    let snap = m.snapshot(&rec.id).unwrap();
    (snap, wall)
}

fn duration_secs(s: u64) -> std::time::Duration {
    std::time::Duration::from_secs(s)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn compress_drill() {
    if std::env::var("IUM_COMPRESS").as_deref() != Ok("1") {
        println!(
            "SKIPPED (loud, #53): set IUM_COMPRESS=1 (+ IUM_TUS_ENDPOINT, the \
             tus-test.sh pattern) to run the compress-mode drill"
        );
        return;
    }
    let endpoint = std::env::var("IUM_TUS_ENDPOINT").expect("IUM_TUS_ENDPOINT (tusd)");

    // --- the compressible leg: wire < source, both hashes, archive exact ---
    let dir = tempfile::tempdir().expect("tempdir");
    let src = write_source(dir.path(), "text.bin", &mk_text(SIZE as usize));
    let (snap, wall) = run_job(&endpoint, &src, SIZE).await;
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        SIZE,
        "source-space confirms sum to the source size (the projection held)"
    );
    let manifest = last_event(&snap.events, "probe")
        .and_then(|p| p["session_token"].as_str().map(String::from))
        .map(|t| serde_json::from_str::<serde_json::Value>(&t).unwrap())
        .expect("the manifest journaled in the probe event");
    assert_eq!(manifest["mode"], "compressed");
    assert_eq!(manifest["source_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(manifest["archive_sha256"].as_str().unwrap().len(), 64);
    let archive_size = manifest["archive_size"].as_u64().unwrap();
    assert!(
        archive_size < SIZE / 2,
        "Cause-D on the record: wire {archive_size} B << source {SIZE} B"
    );
    println!(
        "compress leg: {SIZE} B → {archive_size} B on the wire ({:.1}% saved) in {wall:.2}s",
        100.0 * (1.0 - archive_size as f64 / SIZE as f64)
    );

    // the relay's decode job, over the RECEIVED archive (tusd GET)
    let inner_url = manifest["inner"].as_str().expect("inner token journaled");
    let wire_archive = readback(inner_url).await;
    assert_eq!(
        wire_archive.len() as u64,
        archive_size,
        "the server holds exactly the archive"
    );
    let decoded = {
        let mut dec = zstd::stream::Decoder::new(&wire_archive[..]).unwrap();
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        out
    };
    let source = std::fs::read(&src).unwrap();
    assert_eq!(
        decoded, source,
        "decoded archive is BYTE-EXACT vs the source"
    );

    // the verify detail names the destination honesty
    let verify_detail = last_event(&snap.events, "verify_passed")
        .and_then(|v| v["detail"].as_str().map(String::from))
        .unwrap_or_default();
    if !verify_detail.is_empty() {
        println!("verify: {verify_detail}");
    }

    // D6: Done removed the staging archive
    assert!(
        !staging_archive(&ium_core::JobSpec {
            source_path: src.to_string_lossy().into_owned(),
            source_size: SIZE,
            adapter: "compress".into(),
            endpoint: endpoint.clone(),
        })
        .exists(),
        "the staging archive is cleaned at Done"
    );

    // --- the hostile leg: the gate declines in seconds, plain job runs ---
    let dir2 = tempfile::tempdir().expect("tempdir");
    let hostile = write_source(dir2.path(), "video.bin", &mk_video(SIZE as usize));
    let (snap2, wall2) = run_job(&endpoint, &hostile, SIZE).await;
    assert_eq!(
        snap2
            .chunks
            .iter()
            .map(|c| c.bytes_confirmed)
            .sum::<u64>()
            .min(SIZE),
        SIZE,
        "the declined leg is a plain job (source-space confirms)"
    );
    let manifest2 = last_event(&snap2.events, "probe")
        .and_then(|p| p["session_token"].as_str().map(String::from))
        .map(|t| serde_json::from_str::<serde_json::Value>(&t).unwrap())
        .expect("the decline manifest journaled");
    assert_eq!(manifest2["mode"], "declined");
    assert_eq!(manifest2["why"], "incompressible");
    assert!(
        manifest2["sample_ratio"].as_f64().unwrap() < 1.15,
        "ratio honesty: {}",
        manifest2["sample_ratio"]
    );
    assert!(
        !staging_archive(&ium_core::JobSpec {
            source_path: hostile.to_string_lossy().into_owned(),
            source_size: SIZE,
            adapter: "compress".into(),
            endpoint: endpoint.clone(),
        })
        .exists(),
        "zero temp files on the decline path"
    );
    println!(
        "decline leg: gate said no in {wall2:.2}s (sample_ratio {:.3}) — plain job, zero temp files",
        manifest2["sample_ratio"].as_f64().unwrap()
    );
}
