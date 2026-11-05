//! Issue #53 — the entropy-gated zstd transfer mode contract tests (RED first).
//!
//! Pure entropy gate + the `CompressAdapter` over a recording resumable mock
//! (the fec.rs pattern). The live-server drill lives in `compress_drill.rs`
//! (`scripts/tus-test.sh --compress`); this file is the compile-gate contract.
//!
//! Probe tests serialize behind PROBE_LOCK: the kill-switch test mutates a
//! process-wide env var, so every probe-dependent test must hold the lock.

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::adapter_compress::{staging_archive, CompressAdapter};
use ium_core::chunk::Lease;
use ium_core::compress::{entropy_gate, sample_bytes};
use ium_core::JobSpec;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

// ---------------------------------------------------------------------------
// deterministic fixtures
// ---------------------------------------------------------------------------
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

fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn spec_for(path: &Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "compress".into(),
        endpoint: "http://mock-endpoint/files/".into(),
    }
}

fn lease(resume_from: u64, end: u64) -> Lease {
    Lease {
        idx: 0,
        begin: 0,
        end,
        resume_from,
        attempt: 0,
    }
}

/// Serializes every test that calls `probe` (the kill-switch test flips a
/// process-wide env; parallel probes would race it). Tokio's async mutex —
/// the guard intentionally spans the test's awaits.
async fn probe_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

// ---------------------------------------------------------------------------
// recording resumable mock — the wrapped lane double (fec.rs pattern)
// ---------------------------------------------------------------------------
#[derive(Default)]
struct Recording {
    probes: Vec<JobSpec>,
    uploads: Vec<(String, u64, u64)>, // (endpoint, resume_from, end)
    reattach: Vec<String>,
}

struct RecordingAdapter {
    log: Mutex<Recording>,
}

impl RecordingAdapter {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            log: Mutex::new(Recording::default()),
        })
    }
    fn probed_specs(&self) -> Vec<JobSpec> {
        self.log.lock().unwrap().probes.clone()
    }
}

#[async_trait::async_trait]
impl TransferAdapter for RecordingAdapter {
    fn name(&self) -> &'static str {
        "recording"
    }
    fn capabilities(&self) -> Capability {
        Capability::new(Capability::RESUMABLE)
    }
    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        self.log.lock().unwrap().probes.push(spec.clone());
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: Some(1),
            session_token: Some(format!("sess-{}", spec.source_size)),
            uniform_block: None,
            concatenation: false,
        })
    }
    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let span = lease.end - lease.resume_from;
        self.log.lock().unwrap().uploads.push((
            spec.endpoint.clone(),
            lease.resume_from,
            lease.end,
        ));
        progress.add(span);
        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: Some("tok-archive".into()),
        })
    }
    async fn verify(&self, _spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        Ok(VerifyReport {
            ok: true,
            detail: Some("mock HEAD ok".into()),
        })
    }
    async fn reattach(&self, endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        self.log.lock().unwrap().reattach.push(endpoint.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 1. the entropy gate (pure)
// ---------------------------------------------------------------------------
#[test]
fn gate_accepts_compressible_text_and_reports_the_ratio() {
    let d = entropy_gate(&sample_bytes(&mk_text(4 << 20)));
    assert!(d.accepted(), "text must be accepted");
    assert!(
        d.projected_ratio() >= 1.15,
        "ratio honesty: {}",
        d.projected_ratio()
    );
    // determinism: the same bytes report the SAME ratio (no fiction)
    let again = entropy_gate(&sample_bytes(&mk_text(4 << 20)));
    assert_eq!(d.projected_ratio(), again.projected_ratio());
}

#[test]
fn gate_declines_video_like_bytes() {
    let d = entropy_gate(&sample_bytes(&mk_video(4 << 20)));
    assert!(!d.accepted(), "high-entropy bytes must decline");
    assert!(d.projected_ratio() < 1.15);
    assert_eq!(d.why(), Some("incompressible"));
}

#[test]
fn gate_samples_head_mid_tail_not_head_only() {
    // compressible ONLY in the middle third — head/tail sampling would miss it
    let mut data = mk_video(3 << 20);
    data[(1 << 20)..(2 << 20)].copy_from_slice(&mk_text(1 << 20));
    let d = entropy_gate(&sample_bytes(&data));
    assert!(
        d.accepted(),
        "the middle sample carries the compressibility: ratio {:.3}",
        d.projected_ratio()
    );
}

#[test]
fn gate_small_files_sample_whole() {
    let d = entropy_gate(&sample_bytes(&mk_text(1000)));
    assert!(d.accepted());
}

#[test]
fn gate_declines_empty_source_nothing_to_gain() {
    let d = entropy_gate(&sample_bytes(&[]));
    assert!(
        !d.accepted(),
        "a 0-byte archive would be BIGGER than the source"
    );
}

// ---------------------------------------------------------------------------
// 2. CompressAdapter — the ACCEPTED path over the mock
// ---------------------------------------------------------------------------
#[tokio::test]
async fn accepted_probe_uploads_the_archive_with_a_full_manifest() {
    let _g = probe_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 2u64 << 20;
    let src = write_fixture(dir.path(), "text.bin", &mk_text(size as usize));
    let inner = RecordingAdapter::new();
    let compress = CompressAdapter::new(inner.clone());
    let spec = spec_for(&src, size);

    let report = compress.probe(&spec).await.unwrap();
    assert!(report.resumable, "badges pass through");
    assert_eq!(
        report.min_chunk_bytes,
        Some(size),
        "min_chunk_bytes = source_size forces the single-chunk shape (D2)"
    );
    assert_eq!(report.max_parallel, Some(1), "one archive session");

    // the inner lane saw the ARCHIVE as its source (smaller, .zst path)
    let probed = inner.probed_specs();
    assert_eq!(probed.len(), 1);
    assert!(probed[0].source_path.ends_with(".zst"));
    assert!(
        probed[0].source_size < size,
        "archive smaller than source: {} < {size}",
        probed[0].source_size
    );

    // the manifest: mode, level, BOTH hashes, both sizes, ratio, inner token
    let v: serde_json::Value = serde_json::from_str(&report.session_token.unwrap()).unwrap();
    assert_eq!(v["mode"], "compressed");
    assert_eq!(v["source_sha256"].as_str().unwrap().len(), 64, "hex sha256");
    assert_eq!(v["archive_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(v["source_size"], size);
    assert_eq!(v["archive_size"], probed[0].source_size);
    assert!(v["ratio"].as_f64().unwrap() > 1.15);
    assert_eq!(
        v["inner"],
        format!("sess-{}", v["archive_size"].as_u64().unwrap()),
        "the inner token rides the manifest (ADR-002)"
    );

    // the staging archive exists and decodes back to the source
    let zst = staging_archive(&spec);
    assert!(zst.exists());
    assert!(
        ium_core::compress::verify_archive(&zst, v["source_sha256"].as_str().unwrap(), size)
            .unwrap(),
        "archive decodes to the journaled source hash"
    );
    // finalize is a no-op (the manager finalizes BEFORE verify — the decode
    // gate needs the bytes); verify-SUCCESS is the cleanup point (D6)
    assert!(compress.finalize(&spec).await.unwrap().is_none());
    assert!(
        zst.exists(),
        "finalize must not eat the verify gate's bytes"
    );
    let ok = compress.verify(&spec).await.unwrap();
    assert!(ok.ok, "intact archive verifies: {:?}", ok.detail);
    assert!(!zst.exists(), "verify-success removes the staging archive");
}

#[tokio::test]
async fn upload_projects_archive_confirms_into_source_space() {
    let _g = probe_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 2u64 << 20;
    let src = write_fixture(dir.path(), "text2.bin", &mk_text(size as usize));
    let inner = RecordingAdapter::new();
    let compress = CompressAdapter::new(inner.clone());
    let spec = spec_for(&src, size);
    let report = compress.probe(&spec).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&report.session_token.unwrap()).unwrap();
    let archive_size = v["archive_size"].as_u64().unwrap();

    // the manager's single chunk spans the whole SOURCE
    let progress = ProgressReporter::new();
    let confirm = compress
        .upload_chunk(&spec, &lease(0, size), &progress)
        .await
        .unwrap();
    assert_eq!(
        confirm.bytes_confirmed, size,
        "the projected confirm sums to the source size (the manager's Done gate)"
    );
    assert_eq!(
        progress.moved(),
        archive_size,
        "wire telemetry counts the ARCHIVE bytes actually moved"
    );
    // the inner drove the full archive span
    assert_eq!(
        inner.log.lock().unwrap().uploads.clone(),
        vec![(spec.endpoint.clone(), 0, archive_size)],
        "one archive upload through the wrapped lane"
    );
}

#[tokio::test]
async fn verify_gates_on_the_archive_and_the_logical_hash() {
    let _g = probe_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 1u64 << 20;
    let src = write_fixture(dir.path(), "text3.bin", &mk_text(size as usize));
    let inner = RecordingAdapter::new();
    let compress = CompressAdapter::new(inner.clone());
    let spec = spec_for(&src, size);
    let report = compress.probe(&spec).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&report.session_token.unwrap()).unwrap();

    // corrupted archive: the checksum gate FAILS the verify (W4) — and the
    // FAILED verify keeps the archive (the #24 repair path re-verifies)
    let zst = staging_archive(&spec);
    let mut bytes = std::fs::read(&zst).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&zst, &bytes).unwrap();
    let bad = compress.verify(&spec).await.unwrap();
    assert!(!bad.ok, "corrupted archive must fail verify loudly");
    assert!(zst.exists(), "a failed verify keeps the bytes for repair");

    // repaired (re-encode): verify passes and the cleanup fires
    ium_core::compress::compress_source(Path::new(&spec.source_path), &zst, 9).unwrap();
    let ok = compress.verify(&spec).await.unwrap();
    assert!(ok.ok, "repaired archive verifies");
    assert!(!zst.exists(), "verify-success removes the staging archive");
    assert!(v["archive_sha256"].as_str().is_some());
}

// ---------------------------------------------------------------------------
// 3. the DECLINE paths — plain jobs with a journaled reason
// ---------------------------------------------------------------------------
#[tokio::test]
async fn declined_probe_is_a_plain_job_with_zero_temp_files() {
    let _g = probe_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 2u64 << 20;
    let src = write_fixture(dir.path(), "video.bin", &mk_video(size as usize));
    let inner = RecordingAdapter::new();
    let compress = CompressAdapter::new(inner.clone());
    let spec = spec_for(&src, size);

    let report = compress.probe(&spec).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&report.session_token.unwrap()).unwrap();
    assert_eq!(v["mode"], "declined");
    assert_eq!(v["why"], "incompressible");
    assert!(v["sample_ratio"].as_f64().unwrap() < 1.15);

    // the inner saw the ORIGINAL spec (plain job, zero behavior change)
    let probed = inner.probed_specs();
    assert_eq!(probed.len(), 1);
    assert_eq!(probed[0].source_path, spec.source_path);
    assert_eq!(probed[0].source_size, size);

    // upload delegates the manager's lease UNCHANGED
    let progress = ProgressReporter::new();
    let confirm = compress
        .upload_chunk(&spec, &lease(0, size), &progress)
        .await
        .unwrap();
    assert_eq!(confirm.bytes_confirmed, size);
    assert_eq!(
        progress.moved(),
        size,
        "wire = source (no parity, no archive)"
    );
    // zero temp files
    assert!(!staging_archive(&spec).exists());
    // verify passes through to the inner
    assert!(compress.verify(&spec).await.unwrap().ok);
}

#[tokio::test]
async fn kill_switch_forces_decline() {
    let _g = probe_lock().await;
    std::env::set_var("IUM_NO_COMPRESS", "1");
    let dir = tempfile::tempdir().unwrap();
    let size = 1u64 << 20;
    let src = write_fixture(dir.path(), "text4.bin", &mk_text(size as usize));
    let inner = RecordingAdapter::new();
    let compress = CompressAdapter::new(inner.clone());
    let spec = spec_for(&src, size);

    let report = compress.probe(&spec).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&report.session_token.unwrap()).unwrap();
    assert_eq!(v["mode"], "declined");
    assert_eq!(v["why"], "IUM_NO_COMPRESS");
    assert!(inner.probed_specs()[0].source_path == spec.source_path);
    std::env::remove_var("IUM_NO_COMPRESS");
}

#[tokio::test]
async fn reattach_replays_the_inner_token() {
    let _g = probe_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 1u64 << 20;
    let src = write_fixture(dir.path(), "text5.bin", &mk_text(size as usize));
    let inner = RecordingAdapter::new();
    let compress = CompressAdapter::new(inner.clone());
    let spec = spec_for(&src, size);
    let report = compress.probe(&spec).await.unwrap();
    let tok = report.session_token.unwrap();
    compress.reattach(&spec.endpoint, &tok).await.unwrap();
    assert_eq!(
        inner.log.lock().unwrap().reattach,
        vec![spec.endpoint.clone()]
    );
}
