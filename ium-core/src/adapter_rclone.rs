//! The rclone bridge lane (issue #63) — 74 backends over the EXISTING S3
//! multipart adapter.
//!
//! rclone v1.75.2 ships 74 storage backends (Google Drive, OneDrive, Dropbox,
//! Mega, Box, pCloud, Koofr, every S3 flavor, WebDAV/FTP/SFTP servers…), and
//! `rclone serve s3` exposes ANY of them over the S3 wire. This adapter owns
//! that serve process and delegates the whole transfer to the proven
//! [`S3MultipartAdapter`] — SigV4 signing, ETag journaling, the ADR-010
//! verify gate: all inherited, zero new S3 code. One lane = breadth day one.
//!
//! Wire facts (rehearsed LIVE 2026-11-05, rclone v1.75.2, `scripts/wirefacts63/`):
//!
//! * **W2/W3** single PUT → `200`, GET back byte-exact.
//! * **W4–W7** multipart over the bridge: init → real `UploadId`; parts →
//!   `200` + real per-part ETags; complete (real ETags) →
//!   `CompleteMultipartUploadResult`; 9 MiB via 2 parts → **byte-exact**.
//!   The server stages parts at `bkt/.rclone_temp_multipart_<id>` and rcat-
//!   materializes on complete — the same shape as IUM's #50 concatenation lane.
//! * **W8** `--auth-key` is BROKEN upstream (both plain and CSV-quoted forms
//!   answer `403 InvalidAccessKeyId`) — the bridge runs the server ANONYMOUS
//!   and binds 127.0.0.1 ONLY. The S3 hop never leaves the loopback; real
//!   authentication is the backend remote's concern (rclone credentials via
//!   env, never a conf file).
//! * **W12** the anonymous server validates signature SHAPE: garbage → 403,
//!   well-formed SigV4 (any creds) → 200 — the inner adapter's real
//!   signatures pass.
//! * **W10** buckets must pre-exist (PUT into an uncreated bucket → 404) —
//!   the bridge runs `rclone mkdir <remote>/<bucket>` before the first probe.
//! * **W11** `--addr 127.0.0.1:0` is the zero-race port: the listen line
//!   echoes the REAL port (`Starting s3 server on [http://127.0.0.1:43803/]`)
//!   and [`parse_listen_port`] reads it.
//!
//! Session semantics (D1, the honest verdict): the S3 multipart session
//! lives in the SERVE process. A fresh adapter in `Connect` mode against the
//! same serve reattaches the journaled UploadId — byte-level resume, the
//! kill-9 shape. A serve death loses the in-memory UploadId registry →
//! `NoSuchUpload` → `ExpiredSession` → the job fails honestly (the #56
//! 24-h-cliff verdict shape; no silent re-upload).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Child;

use crate::adapter::TransferAdapter;
use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, VerifyReport,
};
use crate::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};
use crate::chunk::Lease;
use crate::JobSpec;

/// The bridge's URL scheme — `rclone-bridge://<key>` (the key is required:
/// the journaled endpoint must be self-contained for reattach).
pub const BRIDGE_SCHEME: &str = "rclone-bridge://";

/// How the serve process is owned (D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcloneBridgeMode {
    /// Spawn a fresh `rclone serve s3` on an ephemeral loopback port and
    /// kill it on drop (no orphans — the kill-safety invariant).
    Spawn,
    /// Attach to an already-running serve (operator/supervisor-owned);
    /// never killed by this adapter. `port` must be non-zero.
    Connect {
        /// The loopback port the existing serve listens on (**W11**'s parse
        /// result, carried by the supervisor or a previous spawn).
        port: u16,
    },
}

/// Bridge configuration (adapter-level, never per-job).
#[derive(Debug, Clone)]
pub struct RcloneBridgeConfig {
    /// The rclone binary (default `rclone` on PATH).
    pub rclone_bin: PathBuf,
    /// Spawn vs connect (D1).
    pub mode: RcloneBridgeMode,
    /// The remote/path handed to `rclone serve s3` — a local root for drills
    /// (`/tmp/ium-root`), `gdrive:bucket/path` for real backends.
    pub remote: String,
    /// The single S3 bucket the bridge exposes (default `ium` — the first
    /// path segment of the inner endpoint; ensured with `rclone mkdir`).
    pub bucket: String,
    /// Extra environment for the rclone child (real backends: the
    /// `RCLONE_CONFIG_*` credential variables — env-only, never a conf file).
    pub extra_env: Vec<(String, String)>,
}

impl RcloneBridgeConfig {
    /// Validated constructor (D2): `Connect { port: 0 }` is unusable — loud
    /// error, never guessed.
    pub fn new(mode: RcloneBridgeMode, remote: impl Into<String>) -> Result<Self, AdapterError> {
        let remote = remote.into();
        if remote.trim().is_empty() {
            return Err(AdapterError::Unknown(
                "rclone bridge: empty remote (what would serve s3 serve?)".into(),
            ));
        }
        if let RcloneBridgeMode::Connect { port } = &mode {
            if *port == 0 {
                return Err(AdapterError::Unknown(
                    "rclone bridge: Connect mode needs the serve's real port (got 0)".into(),
                ));
            }
        }
        Ok(Self {
            rclone_bin: PathBuf::from("rclone"),
            mode,
            remote,
            bucket: "ium".into(),
            extra_env: Vec::new(),
        })
    }

    /// Harness/production constructor from the `IUM_RCLONE_*` environment:
    /// `IUM_RCLONE_REMOTE` (required), `IUM_RCLONE_MODE` (`spawn`|`connect`,
    /// default spawn), `IUM_RCLONE_PORT` (required for connect),
    /// `IUM_RCLONE_BIN` (default `rclone`), `IUM_RCLONE_BUCKET` (default `ium`).
    pub fn from_env() -> Result<Self, AdapterError> {
        let remote = std::env::var("IUM_RCLONE_REMOTE")
            .map_err(|_| AdapterError::AuthRequired("IUM_RCLONE_REMOTE not set".into()))?;
        let mode = match std::env::var("IUM_RCLONE_MODE")
            .unwrap_or_default()
            .as_str()
        {
            "" | "spawn" => RcloneBridgeMode::Spawn,
            "connect" => {
                let port: u16 = std::env::var("IUM_RCLONE_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| {
                        AdapterError::Unknown(
                            "IUM_RCLONE_MODE=connect needs IUM_RCLONE_PORT".into(),
                        )
                    })?;
                RcloneBridgeMode::Connect { port }
            }
            other => {
                return Err(AdapterError::Unknown(format!(
                    "IUM_RCLONE_MODE must be spawn|connect, got '{other}'"
                )))
            }
        };
        let mut cfg = Self::new(mode, remote)?;
        if let Ok(bin) = std::env::var("IUM_RCLONE_BIN") {
            cfg.rclone_bin = PathBuf::from(bin);
        }
        if let Ok(b) = std::env::var("IUM_RCLONE_BUCKET") {
            cfg.bucket = b;
        }
        Ok(cfg)
    }
}

/// **W11** — parse the ephemeral port out of rclone's listen line. The v1.75.2
/// notice renders as `Starting s3 server on ttp://127.0.0.1:43803/]` (the
/// leading bracket glitch is rclone's own); the digits after the LAST
/// `127.0.0.1:` before the next non-digit are the port. Hermetically tested.
pub fn parse_listen_port(line: &str) -> Option<u16> {
    const NEEDLE: &str = "Starting s3 server";
    let rest = line.split_once(NEEDLE)?.1;
    const HOST: &str = "127.0.0.1:";
    let after = rest.rsplit_once(HOST)?.1;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// The parsed bridge endpoint — `rclone-bridge://<key>` (D2). The key is
/// REQUIRED and the endpoint must be SELF-CONTAINED: `reattach` receives
/// only the journaled endpoint string (ADR-002) — no spec, no basename
/// fallback — so a bare `rclone-bridge://` could never round-trip a resume
/// (the drill caught exactly that mismatch: reattach rewrote `…/ium/` while
/// verify rewrote `…/ium/<basename>` → inner-map key miss → SessionLost).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEndpoint {
    /// Raw object key (the inner adapter S3-encodes per segment at signing).
    pub key: String,
}

/// Parse the LANE endpoint into the key. `source_path` is accepted for
/// signature parity with the S3 parser but MUST NOT be used to invent a
/// key — the endpoint carries it.
pub fn parse_bridge_endpoint(
    endpoint: &str,
    _source_path: &str,
) -> Result<BridgeEndpoint, AdapterError> {
    let unknown =
        |m: &str| AdapterError::Unknown(format!("bad rclone-bridge endpoint '{endpoint}': {m}"));
    let rest = endpoint
        .strip_prefix(BRIDGE_SCHEME)
        .ok_or_else(|| unknown("scheme must be rclone-bridge://"))?;
    if rest.is_empty() {
        return Err(unknown(
            "the endpoint must carry the object key — rclone-bridge://<key> (reattach round-trips need a self-contained endpoint)",
        ));
    }
    Ok(BridgeEndpoint {
        key: rest.to_string(),
    })
}

/// The deterministic rewrite (D2): the inner adapter speaks
/// `http://{host}/{bucket}/{key}` — the loopback serve, the bridge bucket,
/// the raw key (encoded later by the S3 signer, never here).
pub fn rewrite_endpoint(host: &str, bucket: &str, key: &str) -> String {
    format!("http://{host}/{bucket}/{key}")
}

/// The `rclone_bridge` transfer adapter — owns (or attaches to) one serve
/// process and delegates every transfer call to the inner S3 adapter.
pub struct RcloneBridgeAdapter {
    inner: S3MultipartAdapter,
    host: String,
    bucket: String,
    /// `Some` only in Spawn mode; killed on drop (D1 — no orphans). `Child`
    /// is not `Sync`, so the mutex both grants interior kill access and
    /// makes the adapter `Sync` (the manager shares it across workers).
    child: Mutex<Option<Child>>,
}

impl RcloneBridgeAdapter {
    /// Start (or attach to) the bridge. Spawn mode: binary → serve →
    /// readiness (**W11** parse, 15 s budget) → `rclone mkdir` the bucket
    /// (**W10**) → inner adapter ready. Connect mode: no child work at all.
    pub async fn start(cfg: RcloneBridgeConfig) -> Result<Self, AdapterError> {
        let (host, child) = match &cfg.mode {
            RcloneBridgeMode::Connect { port } => (format!("127.0.0.1:{port}"), None),
            RcloneBridgeMode::Spawn => {
                let mut child = Self::spawn_serve(&cfg).await?;
                let port = Self::wait_ready(&cfg, &mut child).await?;
                (format!("127.0.0.1:{port}"), Some(child))
            }
        };
        if matches!(cfg.mode, RcloneBridgeMode::Spawn) {
            Self::ensure_bucket(&cfg).await?;
        }
        // Throwaway credentials (W12: the anonymous serve accepts well-formed
        // SigV4 from any key and rejects garbage — the inner adapter signs
        // real requests, so it passes; real auth is the remote's, via env).
        let inner = S3MultipartAdapter::new(S3MultipartConfig {
            access_key: "ium-bridge".into(),
            secret_key: "ium-bridge-throwaway".into(),
            region: "us-east-1".into(),
        })?;
        Ok(Self {
            inner,
            host,
            bucket: cfg.bucket.clone(),
            child: Mutex::new(child),
        })
    }

    /// The loopback host:port the inner lane talks to (drill assertions,
    /// supervisor diagnostics).
    pub fn serve_host(&self) -> &str {
        &self.host
    }

    async fn spawn_serve(cfg: &RcloneBridgeConfig) -> Result<Child, AdapterError> {
        tokio::process::Command::new(&cfg.rclone_bin)
            .args(["serve", "s3", &cfg.remote, "--addr", "127.0.0.1:0"])
            .envs(cfg.extra_env.iter().cloned())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            // Belt and braces with Drop::start_kill below: no orphan serves.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                AdapterError::Unknown(format!(
                    "rclone bridge: cannot spawn {}: {e}",
                    cfg.rclone_bin.display()
                ))
            })
    }

    async fn wait_ready(_cfg: &RcloneBridgeConfig, child: &mut Child) -> Result<u16, AdapterError> {
        use tokio::io::AsyncReadExt;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AdapterError::Unknown("rclone bridge: stderr already taken".into()))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut buf = vec![0u8; 4096];
        // Only NEWLINE-COMPLETE lines are scanned: a partial read must never
        // parse a truncated port ("…:438" → 438). Everything after the last
        // '\n' stays pending for the next round.
        let mut complete = String::new();
        let mut pending = String::new();
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                let last = complete.lines().last().unwrap_or(&pending);
                return Err(AdapterError::Unknown(format!(
                    "rclone bridge: no listen line in 15 s — last output: {last}"
                )));
            }
            let remaining = deadline - now;
            match tokio::time::timeout(remaining, stderr.read(&mut buf)).await {
                Err(_) => {
                    return Err(AdapterError::Unknown(
                        "rclone bridge: readiness deadline".into(),
                    ))
                }
                Ok(Err(e)) => {
                    return Err(AdapterError::Unknown(format!("rclone bridge: stderr: {e}")))
                }
                Ok(Ok(0)) => {
                    let last = complete.lines().last().unwrap_or(&pending);
                    return Err(AdapterError::Unknown(format!(
                        "rclone bridge: serve exited early — last output: {last}"
                    )));
                }
                Ok(Ok(n)) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    match pending.rfind('\n') {
                        Some(idx) => {
                            complete.push_str(&pending[..=idx]);
                            pending.drain(..=idx);
                        }
                        None => continue,
                    }
                    for line in complete.lines() {
                        if let Some(port) = parse_listen_port(line) {
                            return Ok(port);
                        }
                    }
                    if complete.len() > 8192 {
                        let keep = complete[complete.len() - 2048..].to_string();
                        complete = keep;
                    }
                }
            }
        }
    }

    async fn ensure_bucket(cfg: &RcloneBridgeConfig) -> Result<(), AdapterError> {
        let target = format!("{}/{}", cfg.remote.trim_end_matches('/'), cfg.bucket);
        let out = tokio::process::Command::new(&cfg.rclone_bin)
            .args(["mkdir", &target])
            .envs(cfg.extra_env.iter().cloned())
            .output()
            .await
            .map_err(|e| AdapterError::Unknown(format!("rclone mkdir {target}: {e}")))?;
        if !out.status.success() {
            return Err(AdapterError::Unknown(format!(
                "rclone mkdir {target} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    /// The inner spec: same job, endpoint rewritten to the loopback serve
    /// (D2). Deterministic — the S3 session map keys stay stable.
    fn inner_spec(&self, spec: &JobSpec) -> Result<JobSpec, AdapterError> {
        let ep = parse_bridge_endpoint(&spec.endpoint, &spec.source_path)?;
        Ok(JobSpec {
            source_path: spec.source_path.clone(),
            source_size: spec.source_size,
            adapter: spec.adapter.clone(),
            endpoint: rewrite_endpoint(&self.host, &self.bucket, &ep.key),
        })
    }
}

/// Spawn-mode drops kill the serve (D1) — `start_kill` is synchronous, so no
/// runtime is needed inside `drop` (the `kill_on_drop(true)` command flag is
/// the second net).
impl Drop for RcloneBridgeAdapter {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(c) = guard.as_mut() {
                let _ = c.start_kill();
            }
        }
    }
}

#[async_trait]
impl TransferAdapter for RcloneBridgeAdapter {
    fn name(&self) -> &'static str {
        "rclone_bridge"
    }

    fn capabilities(&self) -> Capability {
        // Inherited from the S3 lane (part-granularity honesty kept —
        // ADR-004): parts are atomic, chunks re-splittable at probe bounds.
        Capability::new(Capability::RESUMABLE | Capability::DYNAMIC_CHUNKS)
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        let inner = self.inner_spec(spec)?;
        self.inner.probe(&inner).await
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let inner = self.inner_spec(spec)?;
        self.inner.upload_chunk(&inner, lease, progress).await
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let inner = self.inner_spec(spec)?;
        self.inner.verify(&inner).await
    }

    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        // The journaled endpoint is the LANE endpoint — rewritten with THIS
        // adapter's host so the inner map keys line up (the connect-mode
        // resume shape: same serve, same UploadId, byte-level resume). The
        // endpoint is self-contained (the key is REQUIRED — D2): reattach
        // gets no spec, so a bare `rclone-bridge://` is unusable, loud.
        let unknown = |m: &str| {
            AdapterError::Unknown(format!("bad rclone-bridge endpoint '{endpoint}': {m}"))
        };
        let rest = endpoint
            .strip_prefix(BRIDGE_SCHEME)
            .ok_or_else(|| unknown("scheme must be rclone-bridge://"))?;
        if rest.is_empty() {
            return Err(unknown(
                "the endpoint must carry the object key — rclone-bridge://<key>",
            ));
        }
        let inner_ep = rewrite_endpoint(&self.host, &self.bucket, rest);
        self.inner.reattach(&inner_ep, token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::AdapterRegistry;
    use crate::ChunkPool;
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn listen_line_variants() {
        // W11 — v1.75.2's exact render, bracket glitch included.
        let line = "2026/11/05 12:25:52 NOTICE: Local file system at /tmp/wf10/root: \
                    Starting s3 server on ttp://127.0.0.1:43803/]";
        assert_eq!(parse_listen_port(line), Some(43803));
        // A line with a DIFFERENT 127.0.0.1 mention before the listen line
        // must still find the server port (rsplit: the LAST host mention).
        let noisy =
            "NOTICE: 127.0.0.1:1234 was blocked; Starting s3 server on [http://127.0.0.1:9911/]";
        assert_eq!(parse_listen_port(noisy), Some(9911));
        assert_eq!(parse_listen_port("nothing here"), None);
        assert_eq!(
            parse_listen_port("Starting s3 server on [http://127.0.0.1:/]"),
            None
        );
    }

    #[test]
    fn endpoint_contract() {
        // The key is REQUIRED — self-contained endpoints round-trip reattach.
        let e = parse_bridge_endpoint("rclone-bridge://video.mp4", "/tmp/s/other.bin").unwrap();
        assert_eq!(e.key, "video.mp4");
        let e2 = parse_bridge_endpoint("rclone-bridge://sub/dir/clip.bin", "/tmp/s/v.mp4").unwrap();
        assert_eq!(e2.key, "sub/dir/clip.bin");
        assert!(parse_bridge_endpoint("https://x/y", "/tmp/f").is_err());
        // The basename-default footgun is GONE (the drill caught it).
        assert!(parse_bridge_endpoint("rclone-bridge://", "/tmp/s/has-a-basename.bin").is_err());
    }

    #[test]
    fn rewrite_is_deterministic() {
        assert_eq!(
            rewrite_endpoint("127.0.0.1:43803", "ium", "sub/dir/clip.bin"),
            "http://127.0.0.1:43803/ium/sub/dir/clip.bin"
        );
    }

    #[test]
    fn config_validation_is_loud() {
        assert!(RcloneBridgeConfig::new(RcloneBridgeMode::Connect { port: 0 }, "/tmp/r").is_err());
        assert!(RcloneBridgeConfig::new(RcloneBridgeMode::Spawn, "").is_err());
        let cfg = RcloneBridgeConfig::new(RcloneBridgeMode::Spawn, "/tmp/r").unwrap();
        assert_eq!(cfg.bucket, "ium");
        assert_eq!(cfg.rclone_bin, Path::new("rclone"));
    }

    #[test]
    fn from_env_requires_the_remote() {
        // No IUM_RCLONE_REMOTE in the test process → AuthRequired, loud.
        // (The drill lane sets its own env; this is the hermetic branch.)
        if std::env::var("IUM_RCLONE_REMOTE").is_ok() {
            return; // operator env present — the skip is honest, not silent
        }
        match RcloneBridgeConfig::from_env() {
            Err(AdapterError::AuthRequired(m)) => assert!(m.contains("IUM_RCLONE_REMOTE")),
            other => panic!("expected AuthRequired, got {other:?}"),
        }
    }

    fn write_source(dir: &Path, size: u64) -> (PathBuf, Vec<u8>) {
        let path = dir.join("ium-bridge-drill.bin");
        let mut src = vec![0u8; size as usize];
        for (i, b) in src.iter_mut().enumerate() {
            *b = ((i as u64 * 29 + 7) % 251) as u8;
        }
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&src).unwrap();
        (path, src)
    }

    fn drill_bin() -> Option<PathBuf> {
        if std::env::var("IUM_RCLONE_DRILL").unwrap_or_default() != "1" {
            return None;
        }
        let bin = std::env::var("IUM_RCLONE_BIN").unwrap_or_else(|_| "rclone".into());
        let bin = PathBuf::from(bin);
        match std::process::Command::new(&bin).arg("--version").output() {
            Ok(o) if o.status.success() => Some(bin),
            _ => None,
        }
    }

    /// The LIVE drill (D8) — skips loud without `IUM_RCLONE_DRILL=1` and a
    /// working rclone binary. Shape: 12 MiB / 2 parts of 6 MiB, OUT-OF-ORDER
    /// part 1 before part 0, a fresh CONNECT adapter resumes from the
    /// JOURNALED token (the crash-window shape), the ADR-010 verify gate
    /// passes, the object on disk is byte-exact, and the spawned serve is
    /// REAPED on drop (no orphans — the kill-safety invariant).
    #[tokio::test]
    async fn bridge_live_drill() {
        let Some(bin) = drill_bin() else {
            eprintln!("bridge drill: IUM_RCLONE_DRILL=1 + rclone binary required — skip");
            return;
        };
        let mib = 1024u64 * 1024;
        let size: u64 = 12 * mib;
        let dir = std::env::temp_dir().join(format!("ium-bridge-drill-{}", crate::ids::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (path, src) = write_source(&dir, size);

        let mut cfg =
            RcloneBridgeConfig::new(RcloneBridgeMode::Spawn, dir.to_string_lossy().as_ref())
                .unwrap();
        cfg.rclone_bin = bin.clone();
        let a = RcloneBridgeAdapter::start(cfg).await.expect("spawn bridge");
        let port: u16 = a
            .serve_host()
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse().ok())
            .expect("host:port");

        let spec = JobSpec {
            source_path: path.to_string_lossy().into(),
            source_size: size,
            adapter: "rclone_bridge".into(),
            // SELF-CONTAINED endpoint (D2, drill-caught lesson): the key is
            // required — reattach round-trips need it (no spec at reattach).
            endpoint: "rclone-bridge://ium-bridge-drill.bin".into(),
        };

        // W4: probe → resumable S3 report, session token journaled-shape.
        let report = a.probe(&spec).await.expect("probe");
        assert!(report.resumable);
        let token = report.session_token.clone().expect("UploadId token");

        // D8: 2 parts of 6 MiB. Adapter A (the SPAWN owner — it keeps the
        // serve alive) uploads ONLY part 1; the resume of part 0 happens on
        // a FRESH adapter B from the journaled token alone (the supported
        // crash-window shape: connect mode against a supervisor-owned serve).
        let plan = ChunkPool::plan(size, 2, 5 * mib);
        assert_eq!(plan.len(), 2);
        let lease_of = |i: u32| Lease {
            idx: i,
            begin: plan[i as usize].0,
            end: plan[i as usize].1,
            resume_from: plan[i as usize].0,
            attempt: 0,
        };
        let confirm = a
            .upload_chunk(&spec, &lease_of(1), &ProgressReporter::new())
            .await
            .unwrap_or_else(|e| panic!("part 1: {e}"));
        assert_eq!(confirm.bytes_confirmed, plan[1].1 - plan[1].0);
        assert_eq!(confirm.idx, 1);

        // The crash-window shape: a FRESH adapter in CONNECT mode (same
        // serve) reattaches from the JOURNALED token — no re-probe, no
        // orphan session (D1/D7).
        let bcfg = RcloneBridgeConfig::new(
            RcloneBridgeMode::Connect { port },
            dir.to_string_lossy().as_ref(),
        )
        .unwrap();
        let b = RcloneBridgeAdapter::start(bcfg)
            .await
            .expect("connect bridge");
        assert_eq!(b.serve_host(), a.serve_host(), "same serve, same session");
        b.reattach(&spec.endpoint, &token).await.expect("reattach");
        // Byte-level resume: B finishes the MISSING part 0 — the journaled
        // part 1 is never re-uploaded (the kill-9 acceptance shape).
        let confirm = b
            .upload_chunk(&spec, &lease_of(0), &ProgressReporter::new())
            .await
            .unwrap_or_else(|e| panic!("resume part 0: {e}"));
        assert_eq!(confirm.bytes_confirmed, plan[0].1 - plan[0].0);
        let vr = b.verify(&spec).await.expect("verify");
        assert!(vr.ok, "ADR-010 gate must pass: {:?}", vr.detail);

        // Independent byte-compare: the object under the served root is the
        // source (W7 shape — the drill's own eyes, not the adapter's word).
        let object = dir.join("ium").join("ium-bridge-drill.bin");
        let served = std::fs::read(&object).expect("object file");
        assert_eq!(served, src, "byte-exact through the bridge");

        // Registry identity (the lane is pluggable like every other).
        let mut reg = AdapterRegistry::new();
        let rcfg = RcloneBridgeConfig::new(
            RcloneBridgeMode::Connect { port },
            dir.to_string_lossy().as_ref(),
        )
        .unwrap();
        let c = RcloneBridgeAdapter::start(rcfg)
            .await
            .expect("registry instance");
        reg.register(std::sync::Arc::new(c));
        assert_eq!(reg.get("rclone_bridge").unwrap().name(), "rclone_bridge");

        // Kill-safety (D1): dropping the SPAWN adapter reaps the serve.
        drop(b);
        drop(a);
        let mut reaped = false;
        for _ in 0..40 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                reaped = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(reaped, "serve must be reaped after the spawn adapter drops");

        std::fs::remove_dir_all(&dir).ok();
    }
}
