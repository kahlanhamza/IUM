//! Issue #52 — the FEC lane contract tests (RED first).
//!
//! Pure symbolizer/rebuilder + the `fec` pseudo-adapter over a recording
//! resumable mock. The live-server drill lives in `fec_drill.rs`
//! (`scripts/tus-test.sh --fec`); this file is the compile-gate contract.

use ium_core::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use ium_core::adapter_fec::FecAdapter;
use ium_core::chunk::Lease;
use ium_core::fec::{parse_stream, rebuild_group, FecConfig, FecGeometry};
use ium_core::JobSpec;
use raptorq::{EncodingPacket, ObjectTransmissionInformation, SourceBlockEncoder};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// deterministic source generator — the exact generator the W11/W12 rehearsal
// used (scripts/wirefacts52), so the pinned vectors below reproduce byte-exact.
// ---------------------------------------------------------------------------
fn mk_source(len: usize) -> Vec<u8> {
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

// ---------------------------------------------------------------------------
// recording resumable mock — the wrapped lane double
// ---------------------------------------------------------------------------
#[derive(Default)]
struct Recording {
    probes: Vec<String>,              // endpoints probed
    uploads: Vec<(String, u64, u64)>, // (endpoint, resume_from, end)
    reattach: Vec<(String, String)>,
    verifies: Vec<String>,
    drop_probes: usize, // make the Nth probe fail (SessionLost) for the loud-error test
}

struct RecordingAdapter {
    log: Mutex<Recording>,
    session_seq: std::sync::atomic::AtomicU64, // server-like unique session ids
}

impl RecordingAdapter {
    fn new() -> Self {
        Self {
            log: Mutex::new(Recording::default()),
            session_seq: std::sync::atomic::AtomicU64::new(0),
        }
    }
    fn reattach_count(&self) -> usize {
        self.log.lock().unwrap().reattach.len()
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
        let mut log = self.log.lock().unwrap();
        if log.drop_probes > 0 {
            log.drop_probes -= 1;
            return Err(AdapterError::SessionLost(format!(
                "injected at {}",
                spec.endpoint
            )));
        }
        log.probes.push(spec.endpoint.clone());
        let n = self
            .session_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: None,
            min_chunk_bytes: None,
            max_parallel: Some(1),
            // Server-like: every creation yields a UNIQUE session id (tusd's
            // Locations do; the fec lane's groups are told apart by these).
            session_token: Some(format!("sess-{}-{n}", spec.endpoint.trim_end_matches('/'))),
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
            adapter_token: Some(format!("tok-{}", spec.endpoint)),
        })
    }
    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        self.log
            .lock()
            .unwrap()
            .verifies
            .push(spec.endpoint.clone());
        Ok(VerifyReport {
            ok: true,
            detail: Some("mock".into()),
        })
    }
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.log
            .lock()
            .unwrap()
            .reattach
            .push((endpoint.to_string(), token.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------
fn spec_for(path: &Path, size: u64) -> JobSpec {
    JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "fec".into(),
        endpoint: "http://mock-endpoint/files".into(),
    }
}

fn lease(idx: u32, resume_from: u64, end: u64) -> Lease {
    Lease {
        idx,
        begin: 0,
        end,
        resume_from,
        attempt: 0,
    }
}

fn write_source(dir: &Path, size: usize) -> PathBuf {
    let p = dir.join("source.bin");
    std::fs::write(&p, mk_source(size)).unwrap();
    p
}

// ---------------------------------------------------------------------------
// 1. geometry (pure)
// ---------------------------------------------------------------------------
#[test]
fn geometry_default_one_mebibyte_groups() {
    let cfg = FecConfig::default();
    let geo = FecGeometry::derive(&cfg, 10 * 1024 * 1024);
    assert_eq!(geo.group_count(), 10);
    assert_eq!(geo.group_bytes, 64 * 16384); // K_default × symbol_size
    assert_eq!(geo.k(0), 64);
    assert_eq!(geo.r(0), 16); // 25%
    assert_eq!(
        geo.wire_len(0),
        6 + (64 + 16) * (4 + 16384),
        "wire = 6 B header + (K+R) packets of 4 B PayloadId + symbol (W1)"
    );
    // tail group: 10 MiB = 10 exact groups → no partial; try 10.5 MiB
    let geo = FecGeometry::derive(&cfg, 10 * 1024 * 1024 + 512 * 1024);
    assert_eq!(geo.group_count(), 11);
    assert_eq!(geo.k(10), 32, "tail = 512 KiB / 16 KiB");
    assert_eq!(geo.r(10), 8);
}

#[test]
fn geometry_grows_groups_beyond_max() {
    let cfg = FecConfig::default(); // max_groups 4096
                                    // 8 GiB / 1 MiB = 8192 groups > 4096 → G grows to ~2 MiB
    let geo = FecGeometry::derive(&cfg, 8 * 1024 * 1024 * 1024);
    assert!(
        geo.group_count() <= cfg.max_groups,
        "group count capped at max_groups, got {}",
        geo.group_count()
    );
    // groups still symbol-aligned (K divides evenly for full groups)
    assert_eq!(geo.group_bytes % geo.symbol_size as u64, 0);
}

#[test]
fn geometry_empty_file_is_one_zero_group() {
    let geo = FecGeometry::derive(&FecConfig::default(), 0);
    assert_eq!(geo.group_count(), 1);
    assert_eq!(geo.k(0), 0);
    assert_eq!(geo.r(0), 0);
    assert_eq!(geo.wire_len(0), 6, "header-only session");
}

// ---------------------------------------------------------------------------
// 2. framing + symbolize + rebuild (pure)
// ---------------------------------------------------------------------------
#[test]
fn symbolize_frames_header_and_packet_count() {
    let cfg = FecConfig::default();
    let geo = FecGeometry::derive(&cfg, 2 * cfg.group_bytes() + 1000);
    let src = mk_source((2 * geo.group_bytes + 1000) as usize);
    let stream = ium_core::fec::symbolize_group(&geo, 0, &src[..geo.group_bytes as usize]);
    let (true_len, sym, packets) = parse_stream(&stream).unwrap();
    assert_eq!(true_len, geo.group_bytes);
    assert_eq!(sym, cfg.symbol_size);
    assert_eq!(
        packets.len(),
        (geo.k(0) + geo.r(0)) as usize,
        "K source + R repair packets on the wire"
    );
    // ESI sequencing: source 0..K, repair ESIs ≥ K (extended-count rule, W2)
    for (i, p) in packets.iter().enumerate() {
        let esi = p.payload_id().encoding_symbol_id();
        let block = p.payload_id().source_block_number();
        assert_eq!(block, 0, "one block per group session");
        if i < geo.k(0) as usize {
            assert_eq!(esi, i as u32);
        } else {
            assert!(esi >= geo.k(0), "repair ESI must clear the extended region");
        }
    }
    // packet wire size = 4 + symbol_size (W1/W5)
    assert_eq!(stream.len() as u64, geo.wire_len(0));
}

#[test]
fn parse_stream_is_length_guarded() {
    assert!(parse_stream(&[0u8; 5]).is_err(), "< 6 B header");
    let cfg = FecConfig::default();
    let geo = FecGeometry::derive(&cfg, cfg.group_bytes());
    let src = mk_source(geo.group_bytes as usize);
    let mut stream = ium_core::fec::symbolize_group(&geo, 0, &src);
    stream.truncate(stream.len() - 3); // torn tail (a crashed PATCH's realign point)
    assert!(
        parse_stream(&stream).is_err(),
        "partial trailing packet must not parse as packets"
    );
}

#[test]
fn rebuild_survives_any_r_erasures_and_fails_past_them() {
    let cfg = FecConfig::default();
    let geo = FecGeometry::derive(&cfg, cfg.group_bytes());
    let src = mk_source(geo.group_bytes as usize);
    let stream = ium_core::fec::symbolize_group(&geo, 0, &src);
    let (true_len, sym, packets) = parse_stream(&stream).unwrap();
    let r = geo.r(0) as usize;
    let k = geo.k(0) as usize;

    // drop 0..=R at seeded positions — every subset ≥ K rebuilds byte-exact (W3)
    for drop in 0..=r {
        let mut idx: Vec<usize> = (0..packets.len()).collect();
        let mut x = 0x5EED_0000u64 + drop as u64;
        // deterministic partial shuffle
        for i in 0..drop {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let j = i + (x as usize) % (idx.len() - i);
            idx.swap(i, j);
        }
        let kept: Vec<EncodingPacket> = idx[drop..].iter().map(|&i| packets[i].clone()).collect();
        let built = rebuild_group(true_len, sym, &kept).expect("K-of-(K+R) rebuilds");
        assert_eq!(built, src, "drop {drop} of {r}");
    }
    // R+1 erasures: only K-1 left — honest None (W4)
    let kept: Vec<EncodingPacket> = packets[..k + r - (r + 1)].to_vec();
    assert_eq!(kept.len(), k - 1);
    assert!(rebuild_group(true_len, sym, &kept).is_none());
    // exactly K source packets with ZERO repair: still exact (W4)
    let kept: Vec<EncodingPacket> = packets[..k].to_vec();
    assert_eq!(rebuild_group(true_len, sym, &kept).unwrap(), src);
}

#[test]
fn rebuild_trims_padded_tail_to_true_len() {
    let cfg = FecConfig::default();
    let geo = FecGeometry::derive(&cfg, 1000); // one tail group, K = ceil(1000/16384) = 1
    let src = mk_source(1000);
    let stream = ium_core::fec::symbolize_group(&geo, 0, &src);
    let (true_len, sym, packets) = parse_stream(&stream).unwrap();
    assert_eq!(true_len, 1000);
    assert_eq!(geo.k(0), 1);
    let built = rebuild_group(true_len, sym, &packets).unwrap();
    assert_eq!(built.len(), 1000, "padding trimmed to the true length");
    assert_eq!(built, src);
}

#[test]
fn pinned_vectors_raptor_1_8_1() {
    // W11/W12 capture (scripts/wirefacts52, raptorq 1.8.1): block id 7,
    // symbol_size 1024, K=8 source of 8192 B. Packet wire = 4 B PayloadId + symbol.
    let cfg = ObjectTransmissionInformation::new(0, 1024, 0, 1, 1);
    let data = mk_source(8192);
    let enc = SourceBlockEncoder::new2(7, &cfg, &data);
    let source = enc.source_packets();
    assert_eq!(source.len(), 8);
    let p0 = source[0].serialize();
    assert_eq!(
        &p0[..12],
        &[0x07, 0x00, 0x00, 0x00, 0x51, 0xe0, 0x7b, 0x01, 0xe6, 0xf9, 0xba, 0xfc],
        "source packet 0 (ESI 0) — RFC 6330 implementation vector"
    );
    let rep = enc.repair_packets(0, 1);
    assert_eq!(
        rep[0].payload_id().encoding_symbol_id(),
        10,
        "repair ESI = extended(8) = 10 (RFC 6330 systematic table)"
    );
    let r0 = rep[0].serialize();
    assert_eq!(
        &r0[..12],
        &[0x07, 0x00, 0x00, 0x0a, 0x79, 0xd2, 0xef, 0x92, 0x14, 0x0f, 0x6e, 0x07],
        "first repair packet (ESI 10)"
    );
    // determinism (W6): a fresh encoder reproduces the same bytes
    let again = SourceBlockEncoder::new2(7, &cfg, &data).source_packets();
    assert_eq!(again[0].serialize(), p0);
}

// ---------------------------------------------------------------------------
// 3. the fec pseudo-adapter over the recording mock
// ---------------------------------------------------------------------------
#[tokio::test]
async fn probe_creates_group_sessions_and_journals_fec_plan() {
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), 2 * 1024 * 1024 + 4096); // 2 full + tail
    let inner = Arc::new(RecordingAdapter::new());
    let fec = FecAdapter::new(inner.clone(), FecConfig::default());
    let spec = spec_for(&src, 2 * 1024 * 1024 + 4096);

    let report = fec.probe(&spec).await.unwrap();
    assert!(report.resumable, "badges pass through from the lane");
    assert_eq!(
        report.max_chunk_bytes,
        Some(FecConfig::default().group_bytes()),
        "cap = G: one chunk ⊆ one group span"
    );
    assert_eq!(
        report.min_chunk_bytes, report.max_chunk_bytes,
        "floor = G: the resplit freeze"
    );
    assert_eq!(report.max_parallel, None, "independent group sessions");

    // the journaled fec_plan token: geometry rules + one inner token per group
    let tok = report.session_token.expect("session token journaled");
    let v: serde_json::Value = serde_json::from_str(&tok).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(
        v["groups"].as_array().unwrap().len(),
        3,
        "one session per group"
    );

    // the inner lane saw one probe per group — SAME endpoint (wire fact W13:
    // tusd creates only at its base path; isolation is per lane INSTANCE) —
    // with a distinct server-side session id per group.
    let probes = inner.log.lock().unwrap().probes.clone();
    assert_eq!(probes.len(), 3);
    assert!(
        probes.windows(2).all(|w| w[0] == w[1]),
        "one endpoint, N instances"
    );
    let toks = v["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_ne!(toks[0], toks[1], "distinct server sessions per group");
    assert!(toks.iter().all(|t| t.starts_with("sess-")));

    // staging streams exist and parse
    let staging = ium_core::adapter_fec::staging_dir(&spec);
    for g in 0..3u32 {
        let p = ium_core::adapter_fec::staging_path(&spec, g);
        assert!(p.exists(), "staging stream for group {g}");
        let bytes = std::fs::read(&p).unwrap();
        let (true_len, _sym, packets) = parse_stream(&bytes).unwrap();
        let geo = FecGeometry::derive(&FecConfig::default(), 2 * 1024 * 1024 + 4096);
        assert_eq!(true_len, geo.group_true_len(g));
        assert_eq!(packets.len(), (geo.k(g) + geo.r(g)) as usize);
    }
    let _ = staging;
}

#[tokio::test]
async fn upload_confirms_span_honest_source_bytes_only_on_wire_complete_groups() {
    let dir = tempfile::tempdir().unwrap();
    let size = 2u64 * 1024 * 1024 + 4096;
    let src = write_source(dir.path(), size as usize);
    let inner = Arc::new(RecordingAdapter::new());
    let fec = FecAdapter::new(inner.clone(), FecConfig::default());
    let spec = spec_for(&src, size);
    let _ = fec.probe(&spec).await.unwrap();

    // one lease covering the whole file: every group driven to wire-completion
    let progress = ProgressReporter::new();
    let confirm = fec
        .upload_chunk(&spec, &lease(0, 0, size), &progress)
        .await
        .unwrap();
    // wire telemetry = Σ wire_len(g) (the (K+R)/K overhead is REAL wire time)
    let geo = FecGeometry::derive(&FecConfig::default(), size);
    let wire_total: u64 = (0..geo.group_count()).map(|g| geo.wire_len(g)).sum();
    assert_eq!(progress.moved(), wire_total);
    // source-space confirm = the whole lease span (every group completed)
    assert_eq!(confirm.bytes_confirmed, size);

    // partial delivery: a FRESH adapter (empty session state) whose inner is
    // told to fail mid-drive... the honest rule under the mock: a group the
    // inner never confirms contributes ZERO source bytes.
    let inner2 = Arc::new(RecordingAdapter::new());
    let fec2 = FecAdapter::new(inner2.clone(), FecConfig::default());
    // kill the first group's session creation: probe fails loudly
    inner2.log.lock().unwrap().drop_probes = 1;
    let err = fec2.probe(&spec).await.unwrap_err();
    assert!(
        matches!(err, AdapterError::SessionLost(_)),
        "session creation failure surfaces loudly, not silently: {err:?}"
    );

    // resumed lease: [resume_from, end) confirms only the tail group's span
    // (groups 0..2 already wire-complete from the first drive — the adapter
    // re-derives completion from the inner session, not from optimism).
    let confirm2 = fec
        .upload_chunk(&spec, &lease(1, size / 2, size), &progress)
        .await
        .unwrap();
    assert_eq!(
        confirm2.bytes_confirmed,
        size - size / 2,
        "span-honest: only [resume_from, end) confirmed"
    );
}

#[tokio::test]
async fn reattach_replays_every_group_token() {
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), 1024 * 1024);
    let inner = Arc::new(RecordingAdapter::new());
    let fec = FecAdapter::new(inner.clone(), FecConfig::default());
    let spec = spec_for(&src, 1024 * 1024);
    let report = fec.probe(&spec).await.unwrap();
    let tok = report.session_token.unwrap();

    fec.reattach(&spec.endpoint, &tok).await.unwrap();
    assert_eq!(inner.reattach_count(), 1, "one replay per group session");
}

#[tokio::test]
async fn verify_ands_groups_and_finalize_removes_staging() {
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), 1024 * 1024);
    let inner = Arc::new(RecordingAdapter::new());
    let fec = FecAdapter::new(inner.clone(), FecConfig::default());
    let spec = spec_for(&src, 1024 * 1024);
    let _ = fec.probe(&spec).await.unwrap();

    let v = fec.verify(&spec).await.unwrap();
    assert!(v.ok);
    assert!(
        v.detail.unwrap().contains("relay"),
        "the verify detail states the relay honesty (destination decode is the relay's job)"
    );
    assert_eq!(inner.log.lock().unwrap().verifies.len(), 1);

    // finalize: staging removed, no protocol finalization (read-only verify keeps)
    let staging = ium_core::adapter_fec::staging_dir(&spec);
    assert!(staging.exists());
    let url = fec.finalize(&spec).await.unwrap();
    assert!(url.is_none());
    assert!(
        !staging.exists(),
        "staging cleaned at the all-Confirmed boundary"
    );
}

#[tokio::test]
async fn end_to_end_rebuild_byte_exact_with_r_erasures_per_group() {
    // THE acceptance: fec pseudo-adapter end-to-end, byte-exact with R symbols lost.
    let dir = tempfile::tempdir().unwrap();
    let size = 3u64 * 1024 * 1024 + 777_777; // 3 full groups + a non-aligned tail
    let src = write_source(dir.path(), size as usize);
    let inner = Arc::new(RecordingAdapter::new());
    let fec = FecAdapter::new(inner.clone(), FecConfig::default());
    let spec = spec_for(&src, size);
    let _ = fec.probe(&spec).await.unwrap();

    // drive the whole job in chunk-sized pieces (cap = G, so pieces ⊆ group spans)
    let geo = FecGeometry::derive(&FecConfig::default(), size);
    let mut piece = 0u64;
    let mut idx = 0u32;
    while piece < size {
        let end = (piece + geo.group_bytes).min(size);
        let progress = ProgressReporter::new();
        let c = fec
            .upload_chunk(&spec, &lease(idx, piece, end), &progress)
            .await
            .unwrap();
        assert_eq!(c.bytes_confirmed, end - piece, "piece {idx} completes");
        piece = end;
        idx += 1;
    }

    // the relay's job, proven client-side: for EVERY group, erase R packets
    // (seeded), rebuild, concat, trim — byte-exact vs the source.
    let mut rebuilt: Vec<u8> = Vec::with_capacity(size as usize);
    for g in 0..geo.group_count() {
        let bytes = std::fs::read(ium_core::adapter_fec::staging_path(&spec, g)).unwrap();
        let (true_len, sym, packets) = parse_stream(&bytes).unwrap();
        let r = geo.r(g) as usize;
        let mut idxs: Vec<usize> = (0..packets.len()).collect();
        let mut x = 0x5EED_0052u64 + g as u64;
        for i in 0..r {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let j = i + (x as usize) % (idxs.len() - i);
            idxs.swap(i, j);
        }
        let kept: Vec<EncodingPacket> = idxs[r..].iter().map(|&i| packets[i].clone()).collect();
        rebuilt.extend(rebuild_group(true_len, sym, &kept).unwrap());
    }
    let original = std::fs::read(&src).unwrap();
    assert_eq!(
        rebuilt, original,
        "K-of-(K+R) across ALL groups, R erasures each"
    );
}

#[tokio::test]
async fn zero_byte_file_is_a_header_only_session_that_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), 1); // placeholder content; the job sees size 0
    let inner = Arc::new(RecordingAdapter::new());
    let fec = FecAdapter::new(inner.clone(), FecConfig::default());
    let spec = spec_for(&src, 0);
    let report = fec.probe(&spec).await.unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&report.session_token.clone().unwrap()).unwrap();
    assert_eq!(v["groups"].as_array().unwrap().len(), 1);
    assert_eq!(v["k_default"], 0, "empty group carries no symbols");

    let progress = ProgressReporter::new();
    let c = fec
        .upload_chunk(&spec, &lease(0, 0, 0), &progress)
        .await
        .unwrap();
    assert_eq!(c.bytes_confirmed, 0, "0 source bytes — but the session ran");
    assert_eq!(progress.moved(), 6, "header-only wire traffic");
    assert!(fec.verify(&spec).await.unwrap().ok);
}
