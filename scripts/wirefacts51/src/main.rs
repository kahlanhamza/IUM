//! wire-facts rehearsal for issue #51 (multi-path bonding).
//! Pins, against THIS sandbox, the protocol/OS facts the design depends on:
//!   P1 interface enumeration shape (/sys/class/net + operstate)
//!   P2 SO_BINDTODEVICE privilege requirement as non-root (expect EPERM)
//!   P3 source-address dual-path over loopback (127.0.0.1 vs 127.0.0.2),
//!      server sees distinct peers -> two real paths without root
//!   P4 aggregate-vs-single wall-time on the two source-bound paths

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn p1_enumerate() {
    println!("== P1 /sys/class/net enumeration ==");
    let dir = match std::fs::read_dir("/sys/class/net") {
        Ok(d) => d,
        Err(e) => {
            println!("  FAIL: {e}");
            return;
        }
    };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let state = std::fs::read_to_string(e.path().join("operstate"))
            .unwrap_or_default()
            .trim()
            .to_string();
        println!("  iface={name} operstate={state}");
    }
}

fn bind_to_device(iface: &str) -> std::io::Result<Socket> {
    let s = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    SockRef::from(&s).bind_device(Some(iface.as_bytes()))?;
    Ok(s)
}

// getuid without the libc crate: read /proc/self/status
fn get_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(String::from))
        })
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn p2_so_bindtodevice() {
    println!("== P2 SO_BINDTODEVICE('lo') as uid {} ==", get_uid());
    match bind_to_device("lo") {
        Ok(_) => println!("  FACT: SO_BINDTODEVICE lo SUCCEEDED without privilege"),
        Err(e) => println!("  FACT: SO_BINDTODEVICE lo FAILED: {e} (capability-gated)"),
    }
}

fn source_bound(local: Ipv4Addr, peer: SocketAddr) -> std::io::Result<TcpStream> {
    let s = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    s.set_nonblocking(false)?;
    let addr = std::net::SocketAddr::from((local, 0));
    s.bind(&addr.into())?;
    s.connect(&peer.into())?;
    Ok(s.into())
}

fn p3_dual_path(peer: SocketAddr) {
    println!("== P3 source-bind 127.0.0.1 vs 127.0.0.2 -> {peer} ==");
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let srv = TcpListener::bind("127.0.0.1:0").expect("listener");
    let peer_actual = srv.local_addr().unwrap();
    let t = std::thread::spawn(move || {
        // exactly 2 connections expected, then exit so join() returns
        for _ in 0..2 {
            let Some(Ok(mut c)) = srv.incoming().next() else { break };
            let peer = c.peer_addr().unwrap();
            seen2.lock().unwrap().push(peer.ip().to_string());
            c.write_all(b"ok").unwrap();
        }
    });
    let mut results = Vec::new();
    for local in [Ipv4Addr::new(127, 0, 0, 1), Ipv4Addr::new(127, 0, 0, 2)] {
        match source_bound(local, peer_actual) {
            Ok(mut s) => {
                let mut buf = [0u8; 2];
                s.read_exact(&mut buf).unwrap();
                results.push(format!("{local} -> CONNECT OK"));
            }
            Err(e) => results.push(format!("{local} -> FAIL {e}")),
        }
    }
    for r in results {
        println!("  {r}");
    }
    t.join().ok(); // server thread keeps the listener alive till we drop clients
    let seen = seen.lock().unwrap();
    println!("  server saw peers: {seen:?}");
    let distinct = seen.iter().collect::<std::collections::BTreeSet<_>>().len();
    println!(
        "  FACT: {} distinct source paths proven {}",
        distinct,
        if distinct == 2 { "(DUAL-PATH OK)" } else { "(NOT DISTINCT)" }
    );
}

fn pump(local: Ipv4Addr, peer: SocketAddr, bytes: u64) -> std::io::Result<u64> {
    let mut s = source_bound(local, peer)?;
    let buf = vec![0xABu8; 256 * 1024];
    let mut left = bytes;
    while left > 0 {
        let n = buf.len().min(left as usize) as u64;
        s.write_all(&buf[..n as usize])?;
        left -= n;
    }
    // half-close so the sink sees EOF and frees the thread even while the
    // other path is still pumping
    s.shutdown(std::net::Shutdown::Write)?;
    let mut eof = [0u8; 1];
    let _ = s.read(&mut eof)?; // read EOF back
    Ok(bytes)
}

fn sink_server() -> SocketAddr {
    let srv = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = srv.local_addr().unwrap();
    std::thread::spawn(move || {
        // thread per connection: dual-path must drain in PARALLEL, or P4 lies
        for c in srv.incoming() {
            std::thread::spawn(move || {
                let Ok(mut c) = c else { return };
                let mut sink = [0u8; 64 * 1024];
                loop {
                    match c.read(&mut sink) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
        }
    });
    addr
}

fn p4_aggregate(peer: SocketAddr) {
    let per_path: u64 = 64 * 1024 * 1024; // 64 MiB
    println!("== P4 aggregate vs single ({} MiB per path) ==", per_path >> 20);

    let t0 = Instant::now();
    let single = pump(Ipv4Addr::new(127, 0, 0, 1), peer, per_path).unwrap();
    let t_single = t0.elapsed();

    let t1 = Instant::now();
    let h = std::thread::spawn(move || pump(Ipv4Addr::new(127, 0, 0, 2), peer, per_path).unwrap());
    let a = pump(Ipv4Addr::new(127, 0, 0, 1), peer, per_path).unwrap();
    let b = h.join().unwrap();
    let t_dual = t1.elapsed();

    let agg = a + b;
    let single_mibps = single as f64 / t_single.as_secs_f64() / (1 << 20) as f64;
    let agg_mibps = agg as f64 / t_dual.as_secs_f64() / (1 << 20) as f64;
    println!("  single-path : {single} B in {t_single:?} = {single_mibps:.1} MiB/s");
    println!("  dual-path   : {agg} B in {t_dual:?} = {agg_mibps:.1} MiB/s (aggregate)");
    println!(
        "  FACT: dual/aggregate ratio = {:.2}x on loopback (parallelism, not bandwidth)",
        agg_mibps / single_mibps
    );
}

fn main() {
    p1_enumerate();
    p2_so_bindtodevice();
    let sink = sink_server();
    p3_dual_path(sink);
    p4_aggregate(sink);
    println!("== rehearsal complete ==");
}
