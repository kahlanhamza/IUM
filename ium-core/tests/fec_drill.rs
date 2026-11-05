//! The FEC loss-ladder drill (issue #52 — the honest crossover measurement).
//!
//! What this drill measures (design note §5/§7 — the boundary is on the
//! record BEFORE the numbers): the #32 loopback loss model is RTO-scale
//! STALLS — a userspace relay cannot erase bytes inside a TCP stream — so
//! parity buys nothing HERE: both lanes complete, the fec lane pays the
//! (K+R)/K wire overhead. What the drill PROVES along the way is the
//! erasure-repair capability the relay mode needs: after every fec leg, the
//! RECEIVED group streams are read back from the server, R packets per group
//! are erased (seeded), and the source is rebuilt byte-exact — the relay's
//! decode job, proven over the real wire.
//!
//! Ladder: loss ∈ {0, 3, 5}% × {plain tus lane, fec lane}, same 4 MiB file,
//! same 64 KiB relay segments (`--loss` models one RTO-scale stall per
//! affected segment). Wire accounting: the fec lane's `fec_plan` journal
//! payload carries `total_wire`/`overhead` (≈ 1.25×); the plain lane moves
//! exactly SIZE. Proxy `LOSS stalls=` lines are the stall evidence.
//!
//! Env-gated (`IUM_FEC=1` + `IUM_TUS_ENDPOINT`) — the PR lane skips LOUDLY.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ium_core::adapter::TransferAdapter;
use ium_core::fec::{parse_stream, rebuild_group, FecConfig, FecGeometry};
use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::{FecAdapter, JobSpec, TusAdapter};
use raptorq::EncodingPacket;

const KIB: u64 = 1024;
const SIZE: u64 = 4 * 1024 * KIB; // 4 MiB = 4 groups of the default 1 MiB
const LADDER: [u64; 3] = [0, 3, 5];

struct ProxyGuard(Child);

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn one `--loss PCT` proxy in front of tusd; returns (guard, URL, log).
fn spawn_loss_proxy(tusd_port: u16, pct: u64) -> (ProxyGuard, String, std::path::PathBuf) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let proxy = std::env::var("IUM_PROXY").unwrap_or_else(|_| {
        format!(
            "{}/../scripts/throttle_proxy.py",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let python = std::env::var("IUM_PYTHON").unwrap_or_else(|_| "python3".into());
    let log = std::env::temp_dir().join(format!("ium-fec-drill-loss{pct}-proxy.log"));
    let log_file = std::fs::File::create(&log).expect("proxy log file");
    let child = Command::new(python)
        // -u: unbuffered stdout — the LOSS lines must reach the log WHILE the
        // drill runs (a TTY-less python buffers, and the table would lie).
        .arg("-u")
        .arg(&proxy)
        .arg(port.to_string())
        .arg("127.0.0.1")
        .arg(tusd_port.to_string())
        .arg("--loss")
        .arg(pct.to_string())
        .arg("--victim-part")
        .arg("none")
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("throttle proxy spawn ({proxy}): {e}"));
    let guard = ProxyGuard(child);
    let mut ready = false;
    for _ in 0..80 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(ready, "throttle proxy never became reachable on {port}");
    (guard, format!("http://127.0.0.1:{port}/files/"), log)
}

/// The proxy's own stall evidence: `LOSS stalls=<n> extra_s=<t>` lines.
fn parse_loss_log(log: &std::path::Path) -> (usize, f64) {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut count = 0usize;
    let mut seconds = 0.0f64;
    for line in text.lines() {
        if let Some(rest) = line.split("LOSS stalls=").nth(1) {
            let mut it = rest.split_whitespace();
            if let (Some(n), Some(s)) = (it.next(), it.next()) {
                if let (Ok(n), Ok(s)) = (
                    n.parse::<usize>(),
                    s.trim_start_matches("extra_s=").parse::<f64>(),
                ) {
                    count += n;
                    seconds += s;
                }
            }
        }
    }
    (count, seconds)
}

fn write_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    let mut seed = 0x5Eu8;
    let mut left = size;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let n = buf.len().min(left as usize);
        for b in buf[..n].iter_mut() {
            seed = seed.wrapping_mul(31).wrapping_add(17);
            *b = seed;
        }
        f.write_all(&buf[..n]).unwrap();
        left -= n as u64;
    }
    f.sync_all().unwrap();
    path
}

fn last_event(events: &[(i64, String, String)], kind: &str) -> Option<serde_json::Value> {
    events
        .iter()
        .rev()
        .find(|(_, k, _)| k == kind)
        .and_then(|(_, _, p)| serde_json::from_str(p).ok())
}

async fn readback(url: &str) -> Vec<u8> {
    // The read-back rides the SAME lossy proxy as the upload (measurement
    // traffic, not transfer traffic). The proxy latches nothing across
    // connections, so a failed GET retries on a fresh one.
    let mut last_err = None;
    for attempt in 0..3 {
        match reqwest::get(url.to_string()).await {
            Ok(resp) => {
                assert_eq!(resp.status().as_u16(), 200, "read-back {url}");
                match resp.bytes().await {
                    Ok(b) => return b.to_vec(),
                    Err(e) => last_err = Some(e.to_string()),
                }
            }
            Err(e) => last_err = Some(e.to_string()),
        }
        println!(
            "    readback retry {attempt} for {url}: {}",
            last_err.as_deref().unwrap_or("?")
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!(
        "read-back {url} failed 3 attempts: {}",
        last_err.unwrap_or_default()
    );
}

/// Deterministic per-group erasure of exactly R packets (seeded), then the
/// relay's decode job: any K of the rest rebuilds the group byte-exact.
fn erase_and_rebuild(stream: &[u8], r: usize, seed: u64) -> Vec<u8> {
    let (true_len, sym, packets) = parse_stream(stream).expect("framed stream parses");
    assert!(packets.len() > r, "enough packets to erase {r}");
    let mut idx: Vec<usize> = (0..packets.len()).collect();
    let mut x = seed | 1;
    for i in 0..r {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let j = i + (x as usize) % (idx.len() - i);
        idx.swap(i, j);
    }
    let kept: Vec<EncodingPacket> = idx[r..].iter().map(|&i| packets[i].clone()).collect();
    rebuild_group(true_len, sym, &kept).expect("K-of-(K+R) rebuild after R erasures")
}

/// The PLAIN lane leg: same file through the same shaped wire.
async fn run_plain_leg(endpoint: String) -> f64 {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(dir.path(), "plain.bin", SIZE);
    let source = std::fs::read(&path).unwrap();

    let db = dir.path().join("journal.db");
    let mut m = JobManager::open(&db).expect("manager");
    m.register_adapter(Arc::new(TusAdapter::new()));

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SIZE,
        adapter: "tus".into(),
        endpoint,
    };
    let rec = m.create_job(&spec, 4).expect("job");
    let started = Instant::now();
    let done = tokio::time::timeout(Duration::from_secs(240), m.run_job(&rec.id))
        .await
        .expect("plain leg within budget")
        .expect("run ok");
    let wall = started.elapsed().as_secs_f64();
    assert_eq!(done.state, JobState::Done, "plain leg converges Done");

    let snap = m.snapshot(&rec.id).unwrap();
    let probe = last_event(&snap.events, "probe").expect("probe journaled");
    let token = probe["session_token"]
        .as_str()
        .expect("session token journaled")
        .to_string();
    // The managed flow probes with the FLEET seed (#49/#50): against a
    // concatenation-advertising server that token is the concat JSON whose
    // "final" is the materialized upload; a classic probe token is the bare
    // Location. Read back whichever the lane produced.
    let location = if token.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(&token).expect("concat token JSON");
        v["final"]
            .as_str()
            .expect("concat token carries the final URL")
            .to_string()
    } else {
        token
    };
    let got = readback(&location).await;
    assert_eq!(got, source, "plain lane byte-exact read-back");

    println!(
        "    plain: {SIZE} B in {wall:.2}s → {:.1} KiB/s (wire = SIZE, no parity)",
        SIZE as f64 / wall / KIB as f64
    );
    wall
}

/// The FEC lane leg: managed job through the pseudo-adapter; then the relay's
/// decode job proven over the RECEIVED streams (R erasures per group).
async fn run_fec_leg(endpoint: String) -> (f64, f64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(dir.path(), "fec.bin", SIZE);
    let source = std::fs::read(&path).unwrap();

    let db = dir.path().join("journal.db");
    let mut m = JobManager::open(&db).expect("manager");
    // W13: per-group LANE INSTANCES (tusd creates only at /files/, so groups
    // are isolated by instance, not by path — one shared TusAdapter would
    // collapse all four groups into its single endpoint-keyed session slot).
    m.register_adapter(Arc::new(FecAdapter::with_factory(
        Arc::new(|| Arc::new(TusAdapter::new()) as Arc<dyn TransferAdapter>),
        FecConfig::default(),
    )));

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SIZE,
        adapter: "fec".into(),
        endpoint,
    };
    let rec = m.create_job(&spec, 4).expect("job");
    let started = Instant::now();
    let done = tokio::time::timeout(Duration::from_secs(240), m.run_job(&rec.id))
        .await
        .expect("fec leg within budget")
        .expect("run ok");
    let wall = started.elapsed().as_secs_f64();
    assert_eq!(done.state, JobState::Done, "fec leg converges Done");

    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        SIZE,
        "source-space confirms sum to the source size"
    );

    // The journaled fec_plan: geometry + one session token per group.
    let probe = last_event(&snap.events, "probe").expect("probe journaled");
    let token = probe["session_token"].as_str().expect("fec token");
    let plan: serde_json::Value = serde_json::from_str(token).expect("fec token is JSON");
    let geo = FecGeometry::derive(&FecConfig::default(), SIZE);
    assert_eq!(plan["group_count"], geo.group_count(), "one group per MiB");
    let overhead = plan["overhead"].as_f64().expect("overhead journaled");
    assert!(
        (1.2..1.3).contains(&overhead),
        "wire ≈ (K+R)/K × size: overhead {overhead:.4}"
    );

    // The relay's job over the RECEIVED streams: each group's session is read
    // back (tusd GET), R packets erased (seeded), the source rebuilt.
    let groups = plan["groups"].as_array().expect("group tokens journaled");
    assert_eq!(groups.len(), geo.group_count() as usize);
    let mut rebuilt: Vec<u8> = Vec::with_capacity(SIZE as usize);
    for (g, tok) in groups.iter().enumerate() {
        let url = tok.as_str().expect("group session token is a URL");
        assert!(!url.is_empty(), "group {g} has a live session");
        let stream = readback(url).await;
        let r = geo.r(g as u32) as usize;
        rebuilt.extend(erase_and_rebuild(&stream, r, 0x5EED_0052 + g as u64));
    }
    assert_eq!(
        rebuilt, source,
        "byte-exact rebuild of the WHOLE source with R erasures per group"
    );

    println!(
        "    fec:   {SIZE} B in {wall:.2}s → {:.1} KiB/s (wire = {overhead:.4}× SIZE, parity carried)",
        SIZE as f64 / wall / KIB as f64
    );
    (wall, overhead)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fec_loss_ladder_drill() {
    if std::env::var("IUM_FEC").as_deref() != Ok("1") {
        println!(
            "SKIPPED (loud, #52): set IUM_FEC=1 (+ IUM_TUS_ENDPOINT, the \
             tus-test.sh pattern) to run the FEC loss-ladder drill"
        );
        return;
    }
    let endpoint = std::env::var("IUM_TUS_ENDPOINT").expect("IUM_TUS_ENDPOINT (tusd)");
    let tusd: u16 = endpoint
        .split("://")
        .nth(1)
        .expect("authority")
        .split('/')
        .next()
        .expect("host:port")
        .split(':')
        .nth(1)
        .and_then(|p| p.parse().ok())
        .expect("port");

    println!(
        "FEC LOSS LADDER — {SIZE} B, groups = 1 MiB, R = 25% (the honest stall-model boundary)"
    );
    println!(
        "{:>6}  {:>18}  {:>18}  {:>10}  {:>10}",
        "loss%", "plain KiB/s", "fec KiB/s", "fec wire", "stall s"
    );
    for pct in LADDER {
        let (guard, proxied, log) = spawn_loss_proxy(tusd, pct);
        let plain = run_plain_leg(proxied.clone()).await;
        let (fec, overhead) = run_fec_leg(proxied).await;
        drop(guard); // the proxy's log is complete once its process exits
        let (stalls, stall_s) = parse_loss_log(&log);
        println!(
            "{pct:>5}%  {:>18.1}  {:>18.1}  {:>9.4}×  {stalls:>4} / {stall_s:.2}s",
            SIZE as f64 / plain / KIB as f64,
            SIZE as f64 / fec / KIB as f64,
            overhead
        );
    }
    println!(
        "HONEST VERDICT (design note §5): under the #32 stall model parity is pure \
         overhead — both lanes complete, the fec lane pays ≈1.25× wire. Any fec \
         TIME win here is the plain lane's re-drive amplification (stalls × \
         re-PATCHes), NOT parity working — parity repairs ERASURES, and this \
         relay cannot erase TCP bytes. The erasure REPAIR capability \
         (K-of-(K+R), zero round-trips) is proven per leg over the received \
         streams; the crossover flips only on an erasure channel (the relay)."
    );
}
