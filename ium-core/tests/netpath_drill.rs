//! The multi-path bonding drill (issue #51 — the M8 acceptance
//! measurement: the lane must SUM independent bearers).
//!
//! The claim under test: **bonding sums what the paths own** — two
//! independently probed bearers carry twice what one carries, with the
//! SAME session count and the SAME bytes; and a lane with no path plan
//! behaves byte-for-byte like today (the degradation rule).
//!
//! The discriminating shape is the SHARED-BEARER proxy (`--total-rate`):
//! one bearer caps the aggregate no matter how many flows ride it, so
//! * single leg — one bearer, one bond, 2 partials: aggregate ≈ C
//!   (bonding must NOT multiply a bearer it does not own);
//! * dual leg — two bearers, two source-bound bonds (127.0.0.1 /
//!   127.0.0.2 — the no-privilege rig, wire facts P2/P3), 2 partials
//!   split one per bearer: aggregate ≈ 2C.
//!
//! The ratio's classification bar (≥ 1.5×) is the prober's own.
//!
//! Everything REAL: the pinned tusd through per-bearer throttling
//! proxies (`scripts/throttle_proxy.py`), the managed drive loop with
//! `IUM_LINK_PROBE=1` + `IUM_PATHS`, the journaled `path_plan` (weights,
//! exclusions), the concat lane's per-bond partial routing, and the
//! byte-exact read-back. Env-gated (`IUM_MULTIPATH=1` + `IUM_TUS_ENDPOINT`)
//! — the PR lane skips LOUDLY. Run this drill ALONE (it sets the manager's
//! instrument envs); the wall budget ≈ 75 s at the drill's rates.

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ium_core::manager::JobManager;
use ium_core::netpath::NetPath;
use ium_core::state::JobState;
use ium_core::{JobSpec, TusAdapter};

const KIB: u64 = 1024;
const RATE_BPS: u64 = 64 * KIB; // each bearer's shared bucket
const SIZE: u64 = 2 * 1024 * KIB; // 2 MiB per leg: single ≈ 32 s, dual ≈ 16 s

struct ProxyGuard(Child);

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn one shared-bearer proxy in front of tusd; return (guard, URL).
fn spawn_bearer(tusd_port: u16) -> (ProxyGuard, String) {
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
    let child = Command::new(python)
        .arg(&proxy)
        .arg(port.to_string())
        .arg("127.0.0.1")
        .arg(tusd_port.to_string())
        .arg("--total-rate")
        .arg(RATE_BPS.to_string())
        .stdout(Stdio::inherit())
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
    (guard, format!("http://127.0.0.1:{port}/files/"))
}

fn tusd_port(endpoint: &str) -> u16 {
    endpoint
        .split("://")
        .nth(1)
        .expect("endpoint has an authority")
        .split('/')
        .next()
        .expect("authority carries host:port")
        .split(':')
        .nth(1)
        .and_then(|p| p.parse().ok())
        .expect("endpoint carries a port")
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
    let resp = reqwest::get(url.to_string())
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    assert_eq!(resp.status().as_u16(), 200, "read-back {url}");
    resp.bytes().await.unwrap().to_vec()
}

/// One leg: a managed bonded job through the shaped wire, start to Done.
/// `endpoint` is the job's primary endpoint (bearer 0's proxy); the
/// bonds' overrides route their partials to their own bearers.
/// `_guards` keeps the bearer proxies alive for the leg's whole life.
async fn run_leg(
    name: &str,
    endpoint: String,
    bonds: Vec<(NetPath, Option<String>)>,
    seed: u32,
    bearer_count: usize,
    want_plan: bool,
    _guards: &[&ProxyGuard],
) -> f64 {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(dir.path(), "payload.bin", SIZE);
    let source = std::fs::read(&path).unwrap();

    let db = dir.path().join("journal.db");
    let mut m = JobManager::open(&db).expect("manager");
    let adapter = TusAdapter::with_bonds(bonds).expect("bonded adapter builds");
    m.register_adapter(Arc::new(adapter));

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SIZE,
        adapter: "tus".into(),
        endpoint,
    };
    let rec = m.create_job(&spec, seed).expect("job");
    let started = Instant::now();
    let done = tokio::time::timeout(Duration::from_secs(300), m.run_job(&rec.id))
        .await
        .unwrap_or_else(|_| panic!("{name}: the leg outlived its 300 s budget"))
        .unwrap_or_else(|e| panic!("{name}: run error: {e}"));
    let wall = started.elapsed();
    assert_eq!(done.state, JobState::Done, "{name}: must converge Done");

    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        SIZE,
        "{name}: every byte server-acknowledged exactly once"
    );

    // The plan the planner OBEYED (dual leg) / the degradation (single).
    if want_plan {
        let plan = last_event(&snap.events, "path_plan")
            .unwrap_or_else(|| panic!("{name}: path_plan journaled (IUM_PATHS set)"));
        let arr = plan["paths"].as_array().expect("paths array");
        assert_eq!(arr.len(), bearer_count, "{name}: one plan entry per path");
        for p in arr {
            assert_eq!(
                p["excluded"].as_bool(),
                Some(false),
                "{name}: every bearer probed live"
            );
        }
        let probe = last_event(&snap.events, "probe").expect("probe journaled");
        assert_eq!(
            probe["max_parallel"].as_u64().unwrap_or(0),
            seed as u64,
            "{name}: the declaration channel carries the bonded lane"
        );
    } else {
        assert!(
            last_event(&snap.events, "path_plan").is_none(),
            "{name}: no IUM_PATHS → no path_plan (degradation = today's behavior)"
        );
    }

    // Byte-exact read-back — the lane under measurement, materialized.
    // The final URL is journaled on the `concat_final` event (the
    // journal-before-execute gate, #50), not on `done`.
    let final_url = last_event(&snap.events, "concat_final")
        .and_then(|d| {
            d.get("final_url")
                .and_then(|u| u.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| panic!("{name}: the concat lane materialized a final"));
    let got = readback(&final_url).await;
    assert_eq!(got.len() as u64, SIZE, "{name}: read-back length");
    assert_eq!(
        got, source,
        "{name}: byte-exact across {bearer_count} bearer(s)"
    );

    let agg = SIZE as f64 / wall.as_secs_f64();
    println!(
        "{name}: {SIZE} B in {wall:?} → aggregate {:.1} KiB/s across {bearer_count} bearer(s)",
        agg / KIB as f64
    );
    agg
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multipath_drill() {
    if std::env::var("IUM_MULTIPATH").as_deref() != Ok("1") {
        println!(
            "SKIPPED (loud, #51): set IUM_MULTIPATH=1 (+ IUM_TUS_ENDPOINT, \
             the tus-test.sh pattern) to run the bonding drill"
        );
        return;
    }
    let endpoint = std::env::var("IUM_TUS_ENDPOINT").expect("IUM_TUS_ENDPOINT (tusd)");
    let tusd = tusd_port(&endpoint);
    // The manager's instruments: per-path probes + the plan, and the link
    // profile the fleet seed composes from. THIS TEST MUST RUN ALONE —
    // the envs are process-wide (same discipline as #55's drill).
    std::env::set_var("IUM_LINK_PROBE", "1");
    let path_a = NetPath::SourceAddr(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    let path_b = NetPath::SourceAddr(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)));

    // The single-bearer honesty baseline: one bond, 2 partials — the
    // shared bucket must hold the aggregate at ≈ C. No IUM_PATHS: no
    // plan, no composition — degradation is today's behavior.
    std::env::remove_var("IUM_PATHS");
    let (g0, ep0) = spawn_bearer(tusd);
    let single = run_leg(
        "single-bearer",
        ep0.clone(),
        vec![(path_a.clone(), Some(ep0))],
        2,
        1,
        false,
        &[&g0],
    )
    .await;
    drop(g0);

    // The bonded leg: two source-bound paths, each probed, each carrying
    // its partial on its own bearer. Same bytes, same session count.
    std::env::set_var("IUM_PATHS", "addr:127.0.0.1,addr:127.0.0.2");
    let (g1, ep1) = spawn_bearer(tusd);
    let (g2, ep2) = spawn_bearer(tusd);
    let dual = run_leg(
        "dual-bearer",
        ep1.clone(),
        vec![(path_a.clone(), Some(ep1)), (path_b.clone(), Some(ep2))],
        2,
        2,
        true,
        &[&g1, &g2],
    )
    .await;
    drop(g1);
    drop(g2);
    std::env::remove_var("IUM_PATHS");

    let ratio = dual / single;
    println!(
        "BONDED SUM: dual {:.1} KiB/s vs single {:.1} KiB/s = {:.2}× (bar ≥ 1.5×)",
        dual / KIB as f64,
        single / KIB as f64,
        ratio
    );
    assert!(
        ratio >= 1.5,
        "bonding must SUM independent bearers: dual/single = {ratio:.2}× < 1.5×"
    );
}
