//! The link prober (issue #41) — the missing organ between the lanes and
//! the fleet. Nothing in the engine previously MEASURED per-flow vs
//! total-bearer: the fork that decides whether parallelism multiplies.
//! The owner's own link proved both shapes within one hour (~3 KB/s
//! bearer moment vs 1.29 Mbps speedtest) — guessing is malpractice.
//!
//! Method (the owner's research brief §4.1, rate-adaptive per this repo's
//! lane realities — no fixed 8 MiB probe, which at 3 KB/s would run 45
//! minutes):
//!
//! 1. **Single-stream phase**: one canary PATCH (real payload bytes) on
//!    its own tus session → per-flow goodput S1.
//! 2. **K-stream phase**: K independent tus sessions PATCH concurrently
//!    (tus sessions are independent resources — the SESSION is sequential,
//!    not the link) → aggregate goodput SK.
//! 3. **Classify**: SK ≥ 2×S1 → [`LinkClass::PerFlowCap`] (parallelism
//!    multiplies — the IDM shape); SK < 1.5×S1 → [`LinkClass::BearerCap`]
//!    (one bearer — parallelism adds only contention); between →
//!    [`LinkClass::Mixed`], fleet raised cautiously.
//! 4. **Recommend**: piece target ≈ 20 s of measured wire, clamped to the
//!    lane floor (#44's rule: a piece confirms well inside the #42 idle
//!    window); fleet = K when per-flow-capped, 1 when bearer-capped.
//!
//! The profile is DATA, journaled by the caller (`link_profile` event) —
//! the planner consumes recommendations, never guesses (ADR-011's spirit:
//! measure, declare, obey).

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::adapter::ProgressReporter;
use crate::adapter_http::Counting;
use crate::JobSpec;

const TUS_VERSION: &str = "1.0.0";
const TUS_CONTENT_TYPE: &str = "application/offset+octet-stream";
/// A canary is ~10 s of measured wire, clamped to [16 KiB, 1 MiB] — big
/// enough to ride through TCP slow-start, small enough to probe fast.
pub const CANARY_MIN: u64 = 16 * 1024;
/// The canary ceiling: a probe must never move megabytes on a metered
/// bearer (issue #48's "every byte is sacred", shared by the S3 leg).
pub const CANARY_MAX: u64 = 1024 * 1024;

/// How the link answered the 1-vs-K question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkClass {
    /// Aggregate grew with streams: per-flow caps — parallelism multiplies.
    PerFlowCap,
    /// Aggregate flat: one bearer — parallelism only contends.
    BearerCap,
    /// Between the lines: raise cautiously.
    Mixed,
}

/// What one probing pass learned about the link.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LinkProfile {
    /// Single-stream goodput, bytes/second.
    pub single_bps: f64,
    /// K-stream aggregate goodput, bytes/second.
    pub k_bps: f64,
    /// The K the aggregate was measured with.
    pub k: u32,
    /// The classification.
    pub class: LinkClass,
    /// Recommended per-piece bytes (~20 s of measured wire, lane-clamped).
    pub recommended_piece_bytes: u64,
    /// Recommended concurrent streams for the NEXT transfer.
    pub recommended_fleet: u32,
}

/// Probe the link the job will use, with the job's own bytes.
///
/// `source` supplies the canary bytes (the real payload's prefix — never
/// synthetic zeros; ADR-009's honesty applies to probes too). Sessions are
/// created against `endpoint` and abandoned (tusd's expiration or the
/// operator's cleanup owns them; the termination extension is a future
/// negotiation).
pub async fn probe_link(
    source: &Path,
    source_size: u64,
    endpoint: &str,
    k: u32,
    sample_hint_bps: Option<f64>,
) -> std::io::Result<LinkProfile> {
    let k = k.clamp(2, 8);
    // Canary sizing: ~10 s of wire from the hint, else a safe default.
    let canary = sample_hint_bps
        .map(|bps| (10.0 * bps) as u64)
        .unwrap_or(64 * 1024)
        .clamp(CANARY_MIN, CANARY_MAX)
        .min(source_size.max(1));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .build()
        .expect("prober client builds");

    // -- Phase 1: single stream ------------------------------------------------
    let s1 = probe_single_with(&client, source, endpoint, canary).await?;

    // -- Phase 2: K independent sessions, concurrently -------------------------
    // Each session carries canary/K: the AGGREGATE phase moves the same
    // bytes as phase 1, so both phases cost ~the same wall time (never
    // K× the single-stream budget — a 40 s last-stream wait would breach
    // any sane probe timeout on the slow links this exists for).
    let k_canary = (canary / k as u64).max(1024);
    let t1 = Instant::now();
    let mut handles = Vec::with_capacity(k as usize);
    for _i in 0..k {
        let client = client.clone();
        let endpoint = endpoint.to_string();
        let source = source.to_path_buf();
        handles.push(tokio::spawn(async move {
            let session = create_session(&client, &endpoint, k_canary).await?;
            patch_canary(&client, &session, &source, 0, k_canary).await?;
            Ok::<(), std::io::Error>(())
        }));
    }
    for h in handles {
        h.await
            .map_err(|e| std::io::Error::other(format!("prober task: {e}")))??;
    }
    let k_elapsed = t1.elapsed().as_secs_f64().max(1e-9);
    let sk = (k_canary * k as u64) as f64 / k_elapsed;

    // -- Classify + recommend ---------------------------------------------------
    Ok(classify(s1, sk, k, source_size))
}

/// The pure classification core (issue #57): the 1-vs-K question answered
/// from two measurements, shared by BOTH shaped legs — the tus sessions
/// leg and the S3 multipart leg pin the SAME arithmetic. Pure so the
/// contract tests pin the boundaries exactly: ≥ 2× is per-flow (inclusive),
/// < 1.5× is one bearer (exclusive), between is mixed.
/// The piece rule: ~20 s of the measured single-stream wire, clamped to
/// [64 KiB, `CHUNK_MIN`] — and never above the source itself. The lane's
/// own floor (e.g. S3's 5 MiB multipart minimum) applies DOWNSTREAM, in
/// the pool planner (`apply_measured_piece` → `new_capped_floor`): the
/// profile is DATA, not policy.
pub fn classify(s1: f64, sk: f64, k: u32, source_size: u64) -> LinkProfile {
    let (class, fleet) = if sk >= 2.0 * s1 {
        (LinkClass::PerFlowCap, k)
    } else if sk < 1.5 * s1 {
        (LinkClass::BearerCap, 1)
    } else {
        (LinkClass::Mixed, 2)
    };
    let piece = ((20.0 * s1) as u64)
        .clamp(64 * 1024, crate::chunk::CHUNK_MIN)
        .min(source_size.max(1));
    LinkProfile {
        single_bps: s1,
        k_bps: sk,
        k,
        class,
        recommended_piece_bytes: piece,
        recommended_fleet: fleet,
    }
}

/// The K=1 phase against a READY client: one canary session, one PATCH,
/// the single-stream goodput (bytes/second). Shared by `probe_link`'s
/// phase 1 and the per-path prober — the same canary honesty (ADR-009:
/// the payload's own prefix, never synthetic zeros).
async fn probe_single_with(
    client: &reqwest::Client,
    source: &Path,
    endpoint: &str,
    canary: u64,
) -> std::io::Result<f64> {
    let t0 = Instant::now();
    let session = create_session(client, endpoint, canary).await?;
    patch_canary(client, &session, source, 0, canary).await?;
    Ok(canary as f64 / t0.elapsed().as_secs_f64().max(1e-9))
}

/// The per-path prober (issue #51): `probe_link`'s K=1 phase over ONE
/// bound path. A path that cannot carry the canary returns `Err` — the
/// planner maps that to 0 bps, excludes it, and journals
/// `path_excluded`; a live path returns its measured single-stream bps.
///
/// Canary sizing: the same ~10 s-of-wire default as `probe_link` (no
/// hint — the paths are probed before any aggregate knowledge exists).
pub async fn probe_path(
    path: &crate::netpath::NetPath,
    source: &Path,
    source_size: u64,
    endpoint: &str,
) -> std::io::Result<f64> {
    let canary = (64 * 1024)
        .clamp(CANARY_MIN, CANARY_MAX)
        .min(source_size.max(1));
    let client = crate::netpath::client_builder(path)
        .map_err(|e| std::io::Error::other(format!("path client: {e}")))?
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| std::io::Error::other(format!("prober client: {e}")))?;
    probe_single_with(&client, source, endpoint, canary).await
}

async fn create_session(
    client: &reqwest::Client,
    endpoint: &str,
    length: u64,
) -> std::io::Result<String> {
    let resp = client
        .post(endpoint)
        .header("Tus-Resumable", TUS_VERSION)
        .header("Upload-Length", length.to_string())
        .send()
        .await
        .map_err(|e| std::io::Error::other(format!("probe creation failed: {e}")))?;
    if resp.status().as_u16() != 201 {
        return Err(std::io::Error::other(format!(
            "probe creation -> {}",
            resp.status()
        )));
    }
    let raw = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| std::io::Error::other("probe creation without Location"))?
        .to_string();
    Ok(crate::adapter_tus::resolve_location(endpoint, &raw))
}

async fn patch_canary(
    client: &reqwest::Client,
    session: &str,
    source: &Path,
    offset: u64,
    len: u64,
) -> std::io::Result<()> {
    let mut file = tokio::fs::File::open(source).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let reader = file.take(len);
    let progress = ProgressReporter::new();
    let body = reqwest::Body::wrap_stream(FramedRead::new(
        Counting {
            inner: reader,
            progress: progress.clone(),
        },
        BytesCodec::new(),
    ));
    let resp = client
        .patch(session)
        .header("Tus-Resumable", TUS_VERSION)
        .header("Upload-Offset", offset.to_string())
        .header("Content-Type", TUS_CONTENT_TYPE)
        .header("Content-Length", len.to_string())
        .body(body)
        .send()
        .await
        .map_err(|e| std::io::Error::other(format!("probe PATCH failed: {e}")))?;
    if resp.status().as_u16() != 204 {
        return Err(std::io::Error::other(format!(
            "probe PATCH -> {}",
            resp.status()
        )));
    }
    Ok(())
}

/// The journal event name the host records a profile under (the manager's
/// journal is the truth-store; the prober itself stays storage-free).
pub const LINK_PROFILE_EVENT: &str = "link_profile";

/// Build the journaled event payload for a profile (ADR-002: everything
/// the future planner needs, in the journal, never re-probed).
pub fn journal_payload(profile: &LinkProfile) -> serde_json::Value {
    serde_json::json!({
        "single_bps": profile.single_bps,
        "k_bps": profile.k_bps,
        "k": profile.k,
        "class": profile.class,
        "recommended_piece_bytes": profile.recommended_piece_bytes,
        "recommended_fleet": profile.recommended_fleet,
    })
}

/// Convenience: the spec-shaped source description the host already holds.
pub fn spec_for(source: &Path, size: u64, endpoint: &str) -> JobSpec {
    JobSpec {
        source_path: source.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "tus".into(),
        endpoint: endpoint.to_string(),
    }
}
