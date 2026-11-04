//! The aggregate-vs-single measurement drill (issue #55 — the one open
//! acceptance item of #50, the M8 measurement the concat lane still owed).
//!
//! The claim under test: **the concatenation lane rides every bearer a
//! per-flow cap divides** — aggregate > single on a per-flow-capped link,
//! with the prober (#48) classifying the link and the AIMD fleet (#49)
//! adapting within the declared ceiling; and on a shared-bearer link the
//! lane stays fleet-1 (the declaration channel obeys). Everything here is
//! REAL: the pinned tusd through the throttling proxy
//! (`scripts/throttle_proxy.py`, wire facts on the issue from
//! `scripts/issue55_wire_rehearsal.sh`), the managed drive loop with
//! `IUM_LINK_PROBE=1`, the journaled `link_profile`, the concat lane's N
//! partials, the final materialization, and the byte-exact read-back.
//!
//! Three legs (C = T = 64 KiB/s — the sizing math lives on the issue):
//!
//! * **control** — per-flow wire, seed 1: the lane declares ONE flow, so
//!   the aggregate must stay ≈ S1. The instrument's honesty baseline: this
//!   is the shape where a multiplication assertion fails (the RED run
//!   demonstrated exactly that), so the drill measures lane-ness, not
//!   wire-ness.
//! * **per-flow** — per-conn cap C, seed 4: `per_flow_cap`, fleet 4,
//!   `max_parallel: 4` journaled, N=4 partials, **aggregate ≥ 2×S1** (the
//!   prober's own classification bar; the ideal 4× is recorded, and pool
//!   granularity + runner load live between the two — the numbers go on
//!   the record, nothing is hidden).
//! * **bearer** — shared bucket T, seed 4: `bearer_cap`, fleet 1, the
//!   waterline parks workers above 1, **aggregate < 1.5×S1** — parallelism
//!   adds nothing to one bearer, and the lane says so.
//!
//! Env-gated like the ladder: `IUM_CONCAT_MEASURE=1` + `IUM_TUS_ENDPOINT`
//! (tusd, the `tus-test.sh` pattern — the drill is tusd-canonical: the
//! concatenation lane is the thing being measured). The PR lane skips
//! LOUDLY. Wall budget ≈ 90 s at the drill's rates.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ium_core::manager::JobManager;
use ium_core::state::JobState;
use ium_core::JobSpec;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const RATE_BPS: u64 = 64 * KIB; // C and T: the drill's per-flow / bearer rate

/// Kills the shaped wire no matter how the leg ends (a panic in an
/// assertion must not leak a pacing python).
struct ProxyGuard(Child);

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn the throttle proxy in front of tusd and wait for it to answer.
fn spawn_proxy(tusd_port: u16, flags: &[&str]) -> (ProxyGuard, String) {
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
        .arg("--victim-part")
        .arg("none")
        .args(flags)
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
    // http://127.0.0.1:PORT/files/ -> PORT
    let authority = endpoint
        .split("://")
        .nth(1)
        .expect("endpoint has an authority")
        .split('/')
        .next()
        .expect("authority carries host:port");
    authority
        .split(':')
        .nth(1)
        .and_then(|p| p.parse().ok())
        .expect("endpoint carries a port")
}

fn write_source(dir: &std::path::Path, name: &str, size: u64) -> std::path::PathBuf {
    let path = dir.join(name);
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

/// The journaled `link_profile` (the planner's input, not a side channel).
#[derive(Debug, Clone)]
struct Profile {
    single_bps: f64,
    k_bps: f64,
    class: String,
    recommended_fleet: u32,
}

/// What one leg measured and obeyed — everything the assertions judge.
struct LegOutcome {
    profile: Profile,
    max_parallel: u32,
    aggregate_bps: f64,
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

/// One drill leg: a managed job through the shaped wire, start to Done.
async fn run_leg(name: &str, tusd_port: u16, flags: &[&str], seed: u32, size: u64) -> LegOutcome {
    let (_guard, endpoint) = spawn_proxy(tusd_port, flags);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(dir.path(), "payload.bin", size);
    let source = std::fs::read(&path).unwrap();

    let db = dir.path().join("journal.db");
    let mut m = JobManager::open(&db).expect("manager");
    m.register_adapter(std::sync::Arc::new(ium_core::TusAdapter::new()));

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: size,
        adapter: "tus".into(),
        endpoint,
    };
    let rec = m.create_job(&spec, seed).expect("job");
    let started = Instant::now();
    let done = tokio::time::timeout(Duration::from_secs(240), m.run_job(&rec.id))
        .await
        .unwrap_or_else(|_| panic!("{name}: the leg outlived its 240 s budget"))
        .unwrap_or_else(|e| panic!("{name}: run error: {e}"));
    let wall = started.elapsed();
    assert_eq!(done.state, JobState::Done, "{name}: must converge Done");

    let snap = m.snapshot(&rec.id).unwrap();
    assert_eq!(
        snap.chunks.iter().map(|c| c.bytes_confirmed).sum::<u64>(),
        size,
        "{name}: every byte server-acknowledged exactly once"
    );

    // The measurement the planner OBEYED, from the journal (never a side
    // channel): the link profile + the lane's declaration.
    let profile = last_event(&snap.events, "link_profile")
        .unwrap_or_else(|| panic!("{name}: link_profile journaled (IUM_LINK_PROBE=1)"));
    let prof = Profile {
        single_bps: profile["single_bps"].as_f64().expect("single_bps"),
        k_bps: profile["k_bps"].as_f64().expect("k_bps"),
        class: profile["class"].as_str().expect("class").to_string(),
        recommended_fleet: profile["recommended_fleet"].as_u64().expect("fleet") as u32,
    };
    let probe = last_event(&snap.events, "probe").expect("probe journaled");
    let max_parallel = probe["max_parallel"].as_u64().unwrap_or(1) as u32;

    // Byte-exact read-back — the lane under measurement. A concat leg
    // verifies on the FINAL URL (the materialized resource); the classic
    // lane (control, seed 1) journals nothing at finalize — its session
    // URL IS the resource. A JSON token here would mean the lane declared
    // concat but never finalized (a bug the drill must catch, not hide).
    let final_url = last_event(&snap.events, "concat_final")
        .filter(|v| v["state"].as_str() == Some("done"))
        .and_then(|v| v["final_url"].as_str().map(String::from))
        .or_else(|| {
            last_event(&snap.events, "probe")
                .and_then(|v| v["session_token"].as_str().map(String::from))
                .filter(|t| !t.starts_with('{'))
        })
        .unwrap_or_else(|| {
            panic!("{name}: no verifiable URL (concat_final done or classic session)")
        });
    assert_eq!(
        readback(&final_url).await,
        source,
        "{name}: byte-exact through the shaped wire"
    );

    let agg = size as f64 / wall.as_secs_f64();
    eprintln!(
        "[measure] {name}: S1={:.1} KiB/s SK={:.1} KiB/s class={} rec_fleet={} \
         max_parallel={max_parallel} wall={:.1}s aggregate={agg:.1} B/s ({:.1} KiB/s, \
         {:.2}x S1) [{} bytes]",
        prof.single_bps / 1024.0,
        prof.k_bps / 1024.0,
        prof.class,
        prof.recommended_fleet,
        wall.as_secs_f64(),
        agg / 1024.0,
        agg / prof.single_bps,
        size,
    );
    LegOutcome {
        profile: prof,
        max_parallel,
        aggregate_bps: agg,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_concat_lane_multiplies_a_per_flow_cap_and_stays_honest_on_a_bearer() {
    if std::env::var("IUM_CONCAT_MEASURE").as_deref() != Ok("1") {
        eprintln!(
            "skipped (loud, #55): set IUM_CONCAT_MEASURE=1 (+ IUM_TUS_ENDPOINT \
             from tus-test.sh) to run the aggregate-vs-single measurement \
             drill — dispatch/manual; the PR lane runs the compressed lanes"
        );
        return;
    }
    let endpoint = std::env::var("IUM_TUS_ENDPOINT").expect("IUM_TUS_ENDPOINT (tusd)");
    let tusd = tusd_port(&endpoint);
    std::env::set_var("IUM_LINK_PROBE", "1");

    // -- control: one declared flow on the per-flow wire -------------------
    // The instrument's honesty baseline: seed 1 -> the lane stays single,
    // the aggregate rides ONE C. A multiplication assertion evaluated HERE
    // fails (the RED run pinned it) — the drill detects lane-ness.
    let leg = run_leg(
        "control(seed=1, per-flow wire)",
        tusd,
        &["--per-conn-rate", &RATE_BPS.to_string()],
        1,
        2 * MIB,
    )
    .await;
    let (s1, agg) = (leg.profile.single_bps, leg.aggregate_bps);
    assert_eq!(
        leg.profile.class, "per_flow_cap",
        "control: the WIRE is per-flow-capped — the prober must see it even \
         though the lane stays single (the measurement never lies)"
    );
    assert_eq!(
        leg.max_parallel, 1,
        "control: seed 1 -> the classic lane declares exactly one flow"
    );
    assert!(
        agg < 1.5 * s1,
        "control: aggregate {agg:.0} B/s must ride ONE flow (S1={s1:.0}) — \
         a seed-1 lane multiplying would mean the wire, not the lane, is \
         what the drill measures"
    );

    // -- per-flow: the lane rides every bearer the cap divides --------------
    let leg = run_leg(
        "per-flow(seed=4, per-conn cap)",
        tusd,
        &["--per-conn-rate", &RATE_BPS.to_string()],
        4,
        4 * MIB,
    )
    .await;
    let (s1, agg) = (leg.profile.single_bps, leg.aggregate_bps);
    assert_eq!(
        leg.profile.class, "per_flow_cap",
        "per-flow: the prober must classify K·C aggregation"
    );
    assert_eq!(
        leg.profile.recommended_fleet, 4,
        "per-flow: the measurement recommends the fleet it measured"
    );
    assert_eq!(
        leg.max_parallel, 4,
        "per-flow: the concat lane declares 4 partials (ADR-011 channel)"
    );
    assert!(
        agg >= 2.0 * s1,
        "per-flow: aggregate {agg:.0} B/s vs S1 {s1:.0} B/s — the lane must \
         ride the cap's bearers (>= 2x, ideal 4x; the numbers are the record)"
    );

    // -- bearer: the declaration channel obeys the shared bearer ------------
    let leg = run_leg(
        "bearer(seed=4, shared T)",
        tusd,
        &["--total-rate", &RATE_BPS.to_string()],
        4,
        2 * MIB,
    )
    .await;
    let (s1, agg) = (leg.profile.single_bps, leg.aggregate_bps);
    assert_eq!(
        leg.profile.class, "bearer_cap",
        "bearer: the prober must classify the flat aggregate"
    );
    assert_eq!(
        leg.profile.recommended_fleet, 1,
        "bearer: the measurement recommends 1 — parallelism adds nothing"
    );
    assert!(
        agg < 1.5 * s1,
        "bearer: aggregate {agg:.0} B/s vs S1 {s1:.0} B/s — one bearer cannot \
         be multiplied; the waterline must have parked the fleet at 1"
    );
}
