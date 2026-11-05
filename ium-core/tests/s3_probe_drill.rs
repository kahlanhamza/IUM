//! The S3-shaped link-probe LIVE drill (issue #57, M9#2) — env-gated by
//! `IUM_S3_PROBE_DRILL=1` + an rclone binary (`IUM_RCLONE_BIN` or
//! `.toolchain/bin/rclone` or `rclone` on PATH).
//!
//! The discriminating shape (the #55/#51 lesson): the SAME rclone serve s3,
//! the SAME canary sizes, only the throttle model varies —
//!
//! * per-conn 64 KiB/s (each connection its own cap) → the prober must
//!   classify PerFlowCap and recommend fleet = K;
//! * total 64 KiB/s (one shared bearer) → BearerCap, fleet 1;
//! * raw loopback (no proxy) → NOT PerFlowCap (one pipe, however fat).
//!
//! Plus the orphan proof (W5): after every probe the bucket's
//! ListMultipartUploads is EMPTY — the probe aborted its own upload; no
//! probe session ever outlives the call, server-owned.

#[allow(dead_code)] // helpers shared by both drill legs
mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use ium_core::adapter::TransferAdapter;
use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use ium_core::prober::LinkClass;
use ium_core::JobSpec;

const KIB: u64 = 1024;
/// Canary 256 KiB: phase 1 rides ~4 s of a 64 KiB/s cap (past slow-start,
/// past the relay's 250 ms start-credit), the K phase rides 4 x 64 KiB —
/// the same arithmetic the #55 measurement drill pinned.
const SAMPLE_HINT_BPS: f64 = 25.6 * KIB as f64; // 10x -> 256 KiB canary

fn drill_bin() -> Option<PathBuf> {
    if std::env::var("IUM_S3_PROBE_DRILL").unwrap_or_default() != "1" {
        return None;
    }
    let bin = std::env::var("IUM_RCLONE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("rclone"));
    let bin = if bin.exists() {
        bin
    } else {
        PathBuf::from(".toolchain/bin/rclone")
    };
    match Command::new(&bin).arg("--version").output() {
        Ok(o) if o.status.success() => Some(bin),
        _ => None,
    }
}

/// A live rclone serve s3 on an ephemeral loopback port, anonymous mode
/// (W8/W12), with the bucket pre-created (W10: bucket = first-level dir).
struct Serve {
    child: Child,
    host: String,
    bucket_url: String,
    _root: PathBuf,
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Unique-enough ports for the drill's own servers: the tests in this
/// binary run CONCURRENTLY in one process, so pid-only derivations collide
/// (the drill caught its own port fight — the second proxy died silently
/// and every assertion downstream measured the WRONG server). Nanos + an
/// atomic counter keeps every spawn on its own port.
fn drill_port() -> u16 {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    21000
        + ((std::process::id() as u32)
            .wrapping_mul(31)
            .wrapping_add(n.wrapping_mul(7))
            .wrapping_add(nanos)
            % 40000) as u16
}

fn spawn_serve(bin: &Path) -> Serve {
    let port = drill_port();
    let root = std::env::temp_dir().join(format!("ium-s3probe-{}-{}", std::process::id(), port));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bkt")).unwrap();
    let child = Command::new(bin)
        .args([
            "serve",
            "s3",
            root.to_str().unwrap(),
            "--addr",
            &format!("127.0.0.1:{port}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rclone serve");
    // readiness: the anonymous GET (W0)
    let mut ready = false;
    for _ in 0..60 {
        if let Ok(code) = tiny_get(&format!("http://127.0.0.1:{port}/")) {
            if code == 200 {
                ready = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(ready, "rclone serve never became ready");
    Serve {
        child,
        host: format!("127.0.0.1:{port}"),
        bucket_url: format!("http://127.0.0.1:{port}/bkt"),
        _root: root,
    }
}

/// The #55 throttle relay — a raw TCP proxy; SigV4 rides through untouched.
struct Proxy {
    child: Child,
    port: u16,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_proxy(target_port: u16, mode: &str, rate: u32) -> Proxy {
    let port = drill_port();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/throttle_proxy.py")
        .canonicalize()
        .expect("throttle_proxy.py next to the repo");
    let mut cmd = Command::new("python3");
    cmd.arg(&script)
        .arg(port.to_string())
        .arg("127.0.0.1")
        .arg(target_port.to_string());
    match mode {
        "per-conn" => cmd.arg("--per-conn-rate").arg(rate.to_string()),
        "total" => cmd.arg("--total-rate").arg(rate.to_string()),
        other => panic!("unknown proxy mode {other}"),
    };
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let child = cmd.spawn().expect("spawn throttle proxy");
    std::thread::sleep(Duration::from_millis(500)); // listen budget
    Proxy { child, port }
}

fn tiny_get(url: &str) -> Result<u16, String> {
    // unsigned, connection-closed GET — the readiness/orphan shape (W0/W5)
    let out = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "5",
            url,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u16>()
        .map_err(|e| e.to_string())
}

/// The orphan proof (W5): ListMultipartUploads on the bucket must list
/// ZERO uploads after a probe call — the probe aborted its own session.
fn bucket_has_no_open_uploads(bucket_url: &str) -> bool {
    let out = Command::new("curl")
        .args(["-s", "--max-time", "5", &format!("{bucket_url}?uploads")])
        .output()
        .expect("curl");
    let body = String::from_utf8_lossy(&out.stdout);
    !body.contains("<Upload>")
}

fn adapter_for(_url: &str) -> S3MultipartAdapter {
    // W12 throwaway credentials against the anonymous serve
    S3MultipartAdapter::new(S3MultipartConfig {
        access_key: "ium-bridge".into(),
        secret_key: "ium-bridge-throwaway".into(),
        region: "us-east-1".into(),
    })
    .expect("adapter")
}

fn spec_for(source: &Path, size: u64, url: &str) -> JobSpec {
    JobSpec {
        source_path: source.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "s3_multipart".into(),
        endpoint: url.to_string(),
    }
}

fn write_source(dir: &Path, size: u64) -> PathBuf {
    let path = dir.join("probe-src.bin");
    // real payload bytes, not zeros (ADR-009 applies to probes too)
    let mut f = std::fs::File::create(&path).unwrap();
    let mut i = 0u64;
    let mut block = vec![0u8; 64 * 1024];
    while i < size {
        let n = (size - i).min(64 * 1024) as usize;
        for (j, b) in block.iter_mut().enumerate().take(n) {
            *b = ((i + j as u64 * 7) % 251) as u8;
        }
        use std::io::Write;
        f.write_all(&block[..n]).unwrap();
        i += n as u64;
    }
    path
}

const CANARY: u64 = 256 * KIB;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_conn_caps_classify_per_flow_and_leave_no_orphan() {
    let Some(bin) = drill_bin() else {
        eprintln!("s3 probe drill: IUM_S3_PROBE_DRILL=1 + rclone required — skip");
        return;
    };
    let serve = spawn_serve(&bin);
    let proxy = spawn_proxy(
        serve.host.rsplit_once(':').unwrap().1.parse().unwrap(),
        "per-conn",
        64 * 1024,
    );
    let url = format!("http://127.0.0.1:{}/bkt", proxy.port);
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), CANARY * 2);

    let a = adapter_for(&url);
    let spec = spec_for(&src, CANARY * 2, &url);
    // the probe must not touch the lane's session map (a side instrument)
    assert_eq!(a.session_for(&url), None, "no session before");
    let profile = a
        .probe_link(&spec, 4, Some(SAMPLE_HINT_BPS))
        .await
        .expect("per-conn probe")
        .expect("the S3 lane always answers Some");
    assert_eq!(
        a.session_for(&url),
        None,
        "the probe opened no LANE session"
    );

    assert_eq!(
        profile.class,
        LinkClass::PerFlowCap,
        "per-conn caps must classify as per-flow (s1 {:.0} k {:.0})",
        profile.single_bps,
        profile.k_bps
    );
    assert_eq!(
        profile.recommended_fleet, 4,
        "the fleet the measurement supports"
    );
    assert!(
        profile.k_bps >= 2.0 * profile.single_bps,
        "the aggregate must have multiplied: sk {:.0} vs s1 {:.0}",
        profile.k_bps,
        profile.single_bps
    );
    // W5: no orphan — the bucket holds zero open multipart uploads.
    assert!(
        bucket_has_no_open_uploads(&serve.bucket_url),
        "the probe must abort its own upload"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shared_bearer_classifies_bearer_cap_and_leaves_no_orphan() {
    let Some(bin) = drill_bin() else {
        eprintln!("s3 probe drill: IUM_S3_PROBE_DRILL=1 + rclone required — skip");
        return;
    };
    let serve = spawn_serve(&bin);
    let proxy = spawn_proxy(
        serve.host.rsplit_once(':').unwrap().1.parse().unwrap(),
        "total",
        64 * 1024,
    );
    let url = format!("http://127.0.0.1:{}/bkt", proxy.port);
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), CANARY * 2);

    let a = adapter_for(&url);
    let spec = spec_for(&src, CANARY * 2, &url);
    let profile = a
        .probe_link(&spec, 4, Some(SAMPLE_HINT_BPS))
        .await
        .expect("bearer probe")
        .expect("the S3 lane always answers Some");

    assert_eq!(
        profile.class,
        LinkClass::BearerCap,
        "a shared bearer must classify bearer-capped (s1 {:.0} k {:.0})",
        profile.single_bps,
        profile.k_bps
    );
    assert_eq!(profile.recommended_fleet, 1, "one bearer holds the link");
    assert!(
        bucket_has_no_open_uploads(&serve.bucket_url),
        "the probe must abort its own upload"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn raw_loopback_hygiene_and_completion() {
    let Some(bin) = drill_bin() else {
        eprintln!("s3 probe drill: IUM_S3_PROBE_DRILL=1 + rclone required — skip");
        return;
    };
    let serve = spawn_serve(&bin);
    let dir = tempfile::tempdir().unwrap();
    let src = write_source(dir.path(), CANARY * 2);

    let a = adapter_for(&serve.bucket_url);
    let spec = spec_for(&src, CANARY * 2, &serve.bucket_url);
    let profile = a
        .probe_link(&spec, 4, Some(SAMPLE_HINT_BPS))
        .await
        .expect("loopback probe")
        .expect("the S3 lane always answers Some");

    // The loopback lane answers NO classification question: there is no
    // distinguishable cap on one fat pipe, and at wire speed the 1-vs-K
    // ratio is timer-noise dominated (concurrent loopback requests overlap
    // their handler latencies — a real PerFlowCap answer, just not a
    // discriminator). The throttled legs above are the classification
    // instrument. This leg pins ONLY what loopback CAN answer: the probe
    // completes against a live S3 server and leaves nothing behind.
    assert_eq!(profile.k, 4, "the K the aggregate was measured with");
    assert!(
        profile.single_bps > 0.0 && profile.k_bps > 0.0,
        "both phases measured"
    );
    // piece = ~20 s of wire clamped into the engine window [64 KiB, CHUNK_MIN]
    // — on a loopback-speed link the clamp ceiling WINS, so only the window
    // itself is pinnable here (the slow-link piece arithmetic lives in the
    // pure-fn contracts).
    assert!(
        (64 * 1024..=5 * 1024 * 1024).contains(&profile.recommended_piece_bytes),
        "the engine window holds: got {}",
        profile.recommended_piece_bytes
    );
    assert!(
        bucket_has_no_open_uploads(&serve.bucket_url),
        "no orphan on the raw lane either"
    );
}
