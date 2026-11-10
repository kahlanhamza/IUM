//! The freeze drill, parent side (issue #98) — NIGHTLY, Linux, env-gated.
//!
//! The laptop lid closes mid-upload: the engine child is SIGSTOPped with
//! bytes provably in flight (the child announces `INFLIGHT <bytes>` once its
//! watched call has ≥ 64 KiB genuinely on the socket — ProgressReporter
//! truth, not a scheduler guess). The wall clock runs on; SIGCONT wakes the
//! process into a graveyard of dead sockets.
//!
//! WHO kills the wire — the measured fact that shaped this drill (the RED
//! run on the issue, 2026-11-10): SeaweedFS 4.51's gateway does NOT age out
//! a stalled mid-body PUT within 35 s — `-idleTimeout=30` governs idle
//! backend connections, not a quiet-but-open request body (all bytes ACKed,
//! nothing for any timeout to fire on). The first drill draft bet its pins
//! on the server's patience and the wake came back SILENT: the frozen PUT
//! simply completed on thaw, zero `chunk_error`, and pin 2 tripped. On a
//! real lid-close it is the NAT/DHCP churn that kills the mapping — so the
//! drill owns that side of the wire itself: a NAT-twin proxy (std
//! `TcpListener`/`TcpStream` only, zero new dependencies) sits between child
//! and server; ~2 s into the freeze it RST-closes every established
//! connection and refuses new ones until SIGCONT — the honest twin of an
//! expired NAT mapping. The engine must then:
//!
//! * count the death — ≥ 1 `chunk_error` journaled strictly after the freeze
//!   timestamp (journal truth, never silence);
//! * re-send at most the unconfirmed part — S3 parts are atomic, so the
//!   wasted attempt is bounded by physics: `WIRE ≤ SIZE + PART`;
//! * reach honest `Done` — the wake NEVER becomes a false terminal, no
//!   `FAILED` ever appears on the child's stdout.
//!
//! Signals go through the system `kill` binary — std cannot signal and the
//! zero-new-dependencies iron rule stands. Linux-only with a loud skip.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SIZE: u64 = 256 * 1024 * 1024; // 2 parts × 128 MiB, ONE worker — the
                                     // watched part must stay in flight long
                                     // enough for the parent's SIGSTOP (a
                                     // loopback part dies in tens of ms)
const PART: u64 = SIZE / 2;
const FREEZE_SECS: u64 = 35;
/// The NAT expiry lands this deep into the freeze (a real mapping dies some
/// seconds after the lid closes, not at the instant of closing).
const NAT_EXPIRY_SECS: u64 = 2;
/// The parent's overall patience (the child itself must finish long before).
const DEADLINE_SECS: u64 = 420;

fn live_endpoint() -> Option<String> {
    match std::env::var("IUM_S3_ENDPOINT") {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => {
            eprintln!("skipped: IUM_S3_ENDPOINT not set (harness provides the server)");
            None
        }
    }
}

/// Split `http://127.0.0.1:PORT/bucket` without a url crate (zero new deps).
/// Returns (host, port, path-including-leading-slash-or-empty).
fn split_endpoint(endpoint: &str) -> (String, u16, String) {
    let rest = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().expect("endpoint port")),
        None => (authority.to_string(), 80),
    };
    (host, port, path)
}

// ---------------------------------------------------------------------------
// The NAT-twin proxy — the drill's kill side. Std blocking IO only; one
// accept loop + one copier pair per connection; a shared state flips the
// wire dead. A blocked copier is woken by shutting its sockets down (a
// blocking read errors out — no polling loops, no timers).
// ---------------------------------------------------------------------------

struct NatState {
    /// true while the "laptop" is shut: established wire dead, new
    /// connections refused (the expired NAT mapping).
    frozen: AtomicBool,
    /// Every live connection's both halves, so the controller can shutdown
    /// them all in one stroke and unblock every parked copier.
    conns: Mutex<Vec<(TcpStream, TcpStream)>>,
}

impl NatState {
    fn kill_all(&self) {
        self.frozen.store(true, Ordering::SeqCst);
        let conns = self.conns.lock().unwrap();
        for (a, b) in conns.iter() {
            let _ = a.shutdown(Shutdown::Both);
            let _ = b.shutdown(Shutdown::Both);
        }
    }
    fn thaw(&self) {
        self.frozen.store(false, Ordering::SeqCst);
        self.conns.lock().unwrap().clear();
    }
}

fn pump(mut from: TcpStream, mut to: TcpStream, nat: Arc<NatState>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if nat.frozen.load(Ordering::SeqCst) {
            let _ = from.shutdown(Shutdown::Both);
            let _ = to.shutdown(Shutdown::Both);
            return;
        }
        match from.read(&mut buf) {
            Ok(0) => {
                // Half-close from the writer: relay the EOF honestly.
                let _ = to.shutdown(Shutdown::Write);
                return;
            }
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// Boot the NAT-twin in front of `(server_host, server_port)`. Returns the
/// local port the CHILD must talk to. One accept-loop thread lives for the
/// whole drill; copier pairs die with their connections.
fn spawn_nat_proxy(server_host: &str, server_port: u16, nat: Arc<NatState>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind NAT-twin proxy");
    let port = listener.local_addr().unwrap().port();
    let server_host = server_host.to_string();
    std::thread::spawn(move || {
        for client in listener.incoming() {
            if nat.frozen.load(Ordering::SeqCst) {
                // The expired mapping: drop the fresh connection at once.
                drop(client);
                continue;
            }
            let Ok(client) = client else { continue };
            let Ok(server) = TcpStream::connect((server_host.as_str(), server_port)) else {
                let _ = client.shutdown(Shutdown::Both);
                continue;
            };
            {
                // Registry append only — no peek-based pruning: on a blocking
                // socket `peek` PARKS until data or EOF, and a healthy quiet
                // connection would deadlock the accept loop (the RED-session
                // lesson in reverse: the wire is quiet far more often than
                // it is dead).
                let mut conns = nat.conns.lock().unwrap();
                conns.push((
                    client.try_clone().expect("clone client half"),
                    server.try_clone().expect("clone server half"),
                ));
            }
            let nat2 = Arc::clone(&nat);
            let nat3 = Arc::clone(&nat);
            // BOTH directions, or the response never flows: c2s (the request
            // body) AND s2c (the response). The first proxy draft spawned two
            // client-side pumps and the child's very first wire request died
            // with "connection closed before message completed" — the wire
            // fact that found this before any gate could. TcpStream is not
            // Copy — the c2s pair rides try_clone()s, the s2c pair the
            // originals.
            let c2 = client.try_clone().expect("clone client half for c2s pump");
            let s2 = server.try_clone().expect("clone server half for c2s pump");
            std::thread::spawn(move || pump(c2, s2, nat2));
            std::thread::spawn(move || pump(server, client, nat3));
        }
    });
    port
}

fn write_zeros(path: &std::path::Path, size: u64) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    let block = vec![0u8; 1024 * 1024];
    let mut left = size;
    while left > 0 {
        let n = block.len().min(left as usize);
        f.write_all(&block[..n]).unwrap();
        left -= n as usize as u64;
    }
    f.sync_all().unwrap();
}

fn s3_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| k.starts_with("IUM_S3_"))
        .collect()
}

fn signal(pid: u32, sig: &str) {
    let st = Command::new("kill")
        .args([format!("-{sig}"), pid.to_string()])
        .status()
        .expect("the system kill binary (POSIX)");
    assert!(st.success(), "kill -{sig} {pid} failed");
}

#[test]
fn sleep_wake_midbody_counted_transient_resume_honest_done() {
    if std::env::consts::OS != "linux" {
        eprintln!("skipped: the freeze drill is Linux-only (SIGSTOP/SIGCONT contract)");
        return;
    }
    let Some(endpoint) = live_endpoint() else {
        return;
    };
    let (server_host, server_port, path) = split_endpoint(&endpoint);

    // The NAT-twin owns the kill side (the RED-run fact: the server will not
    // do it). The child's whole world — probe, PUTs, verify — rides through
    // the proxy, so the freeze kills the wire it is provably using.
    let nat = Arc::new(NatState {
        frozen: AtomicBool::new(false),
        conns: Mutex::new(Vec::new()),
    });
    let proxy_port = spawn_nat_proxy(&server_host, server_port, Arc::clone(&nat));
    let child_endpoint = format!("http://127.0.0.1:{proxy_port}{path}");
    eprintln!("[freeze] NAT-twin proxy on {proxy_port} → {server_host}:{server_port}{path}");

    let exe = env!("CARGO_BIN_EXE_s3_drill_child");
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("freeze64.bin");
    write_zeros(&src, SIZE);
    let journal = dir.path().join("freeze.journal.db");
    let journal_s = journal.to_str().unwrap();
    let src_s = src.to_str().unwrap();

    let mut child = Command::new(exe)
        .args([
            "upload",
            "--journal",
            journal_s,
            "--file",
            src_s,
            "--workers",
            "1",
            "--freeze-after-parts",
            "1",
            "--freeze-threshold",
            "65536",
            "--freeze-chunk-bytes",
            &PART.to_string(),
        ])
        .env_clear()
        .env("IUM_S3_ENDPOINT", &child_endpoint)
        .envs(s3_env().into_iter().filter(|(k, _)| k != "IUM_S3_ENDPOINT"))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn upload child");
    let pid = child.id();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();

    let deadline = Instant::now() + Duration::from_secs(DEADLINE_SECS);
    let mut froze_at: Option<i64> = None;
    let mut wire: Option<u64> = None;
    let mut done: Option<String> = None;
    let mut failed_lines: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    while done.is_none() {
        assert!(
            Instant::now() < deadline,
            "the freeze child missed the {DEADLINE_SECS}s deadline (frozen={:?})",
            froze_at.is_some()
        );
        let line = match out.next() {
            Some(Ok(l)) => l,
            Some(Err(e)) => panic!("read child stdout: {e}"),
            None => panic!(
                "upload child exited early (frozen={:?}) — check stderr",
                froze_at.is_some()
            ),
        };
        seen.push(line.clone());
        if line.starts_with("INFLIGHT ") && froze_at.is_none() {
            // The freeze provably lands mid-body: the watched call has real
            // bytes in flight. Stop the world, then expire the NAT mapping
            // NAT_EXPIRY_SECS into the freeze — the wake lands in a graveyard
            // of dead sockets (the RED run proved the server would NOT have
            // produced one on its own).
            let moved: u64 = line
                .strip_prefix("INFLIGHT ")
                .expect("INFLIGHT <u64>")
                .parse()
                .expect("parse INFLIGHT");
            assert!(moved >= 65_536, "announced bytes must clear the threshold");
            signal(pid, "STOP");
            froze_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            );
            eprintln!("[freeze] SIGSTOP at INFLIGHT {moved} — sleeping {FREEZE_SECS}s");
            std::thread::sleep(Duration::from_secs(NAT_EXPIRY_SECS));
            nat.kill_all();
            eprintln!("[freeze] NAT mapping expired — established wire dead, new conns refused");
            let remain = FREEZE_SECS - NAT_EXPIRY_SECS;
            std::thread::sleep(Duration::from_secs(remain));
            nat.thaw();
            signal(pid, "CONT");
            eprintln!("[freeze] SIGCONT — the wake begins");
        }
        if let Some(rest) = line.strip_prefix("WIRE ") {
            wire = Some(rest.parse().expect("WIRE <u64>"));
        }
        if let Some(rest) = line.strip_prefix("DONE ") {
            done = Some(rest.to_string());
        }
        if line.contains("FAILED") {
            failed_lines.push(line);
        }
    }
    let _ = child.wait();
    let froze_at = froze_at.unwrap_or_else(|| {
        panic!(
            "the child never announced INFLIGHT — the freeze never landed; lines seen: {:?}",
            seen
        )
    });

    // Pin 1: the wake never became a false terminal.
    assert_eq!(
        done.as_deref(),
        Some("done"),
        "the child must reach honest Done after the wake; FAILED lines: {failed_lines:?}"
    );
    assert!(
        failed_lines.is_empty(),
        "FAILED appeared on the child's stdout: {failed_lines:?}"
    );

    // Pin 2: the deaths were counted, not silent — ≥ 1 chunk_error journaled
    // strictly after the freeze timestamp.
    let conn = rusqlite::Connection::open(&journal).expect("open drill journal");
    let post_freeze_errors: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind = 'chunk_error' AND ts > ?1",
            [froze_at],
            |r| r.get(0),
        )
        .expect("count chunk_error");
    assert!(
        post_freeze_errors >= 1,
        "no chunk_error was journaled after the freeze — the dead sockets came back \
         as silence, not as counted transients"
    );
    eprintln!("[freeze] {post_freeze_errors} counted transient(s) after the wake");

    // Pin 3: the wire waste is bounded by physics — the one frozen worker
    // may waste at most its own part beyond the file itself.
    let wire = wire.expect("WIRE line");
    assert!(
        wire <= SIZE + PART,
        "wire {wire} exceeds SIZE + PART ({}): the wake re-sent confirmed bytes",
        SIZE + PART
    );
    eprintln!(
        "[freeze] wire {wire} ≤ {} — bounded waste holds",
        SIZE + PART
    );

    // Pin 4 (implicit in pin 1's DONE done, asserted over the adapter's own
    // ADR-010 verify gate inside the child — the child fails loud otherwise).
}
