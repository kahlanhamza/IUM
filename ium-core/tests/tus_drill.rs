//! The TUS kill-9 drill, parent side (issue #11) — NIGHTLY, Linux,
//! env-gated. The s3_drill shape with the TUS twist the issue names: the
//! interesting window is kill-after-server-ack-before-journal-write.
//!
//! A 2-chunk TUS upload runs in a REAL child process (real manager, real
//! TusAdapter, real journal, real tusd). Only chunk 0 reaches the server;
//! when tusd ACKs it, the child prints `SERVER_ACK` and then HOLDS the
//! confirmation out of the journal's reach. The parent kills inside that
//! hold — the journal provably holds ZERO confirmed bytes while the server
//! provably holds a full chunk. Then a FRESH child on the same journal runs
//! with a FRESH adapter (empty session map) and must:
//!
//! * recover the journaled creation URL through the manager's
//!   `Action::Reattach` (#12) — map-miss → SessionLost → reattach;
//! * absorb the drift through the 409 HEAD-realign path (#10): the journal
//!   converges to the server offset without re-sending what the server has;
//! * finish `DONE done` via the read-only HEAD verify.
//!
//! Byte-level zero re-upload, the issue's acceptance criterion (made EXACT
//! by #30): the resume child's wire movement must cover exactly the bytes
//! the server lacked. The ONE permitted imperfection — a rejected 409
//! attempt's buffered body flush (#10 telemetry finding) — is a scheduling
//! race (0–48 KiB quiet, 192 KiB on a loaded runner; the old calibrated
//! `WIRE_SLACK = 64 KiB` ceiling raced it in nightly run 41618615182), so it
//! is now MEASURED, not bounded: the child prints `WASTED <bytes>` (the
//! engine's own waste accounting for the rejected attempt's flush) and the
//! parent asserts `WIRE − WASTED == SIZE − SERVER_OFFSET` exactly — every
//! byte the server lacked crossed once, every wasted byte accounted, and a
//! manager that re-sends server-held bytes still fails loudly (re-sent
//! server-held bytes are not waste-reclassifiable).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

const SIZE: u64 = 40 * 1024 * 1024; // 2 chunks × 20 MiB
const CHUNK: u64 = SIZE / 2;

fn live_endpoint() -> Option<String> {
    match std::env::var("IUM_TUS_ENDPOINT") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("skipped: IUM_TUS_ENDPOINT not set (harness provides the server)");
            None
        }
    }
}

fn write_pattern(path: &std::path::Path, size: u64) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    let mut seed = 0x5Du8;
    let block = {
        let mut b = vec![0u8; 1024 * 1024];
        for x in b.iter_mut() {
            seed = seed.wrapping_mul(31).wrapping_add(17);
            *x = seed;
        }
        b
    };
    let mut left = size;
    while left > 0 {
        let n = block.len().min(left as usize);
        f.write_all(&block[..n]).unwrap();
        left -= n as u64;
    }
    f.sync_all().unwrap();
}

fn tus_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| k.starts_with("IUM_TUS_"))
        .collect()
}

#[test]
fn kill9_after_server_ack_before_journal_write_then_realign_resume() {
    let Some(_endpoint) = live_endpoint() else {
        return;
    };
    let exe = env!("CARGO_BIN_EXE_tus_drill_child");
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("drill40.bin");
    write_pattern(&src, SIZE);
    let journal = dir.path().join("drill.journal.db");
    let journal_s = journal.to_str().unwrap();
    let src_s = src.to_str().unwrap();

    // -- child 1: real upload, frozen between server ack and journal write --
    let mut c1 = Command::new(exe)
        .args([
            "upload",
            "--journal",
            journal_s,
            "--file",
            src_s,
            "--hold-ms",
            "8000",
        ])
        .env_clear()
        .envs(tus_env())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn upload child");
    let mut out1 = BufReader::new(c1.stdout.take().unwrap()).lines();

    let server_ack: u64 = loop {
        match out1.next() {
            Some(Ok(line)) => {
                if let Some(v) = line.strip_prefix("SERVER_ACK ") {
                    // The child is now holding the un-journaled confirm.
                    break v.parse().expect("SERVER_ACK <u64>");
                }
                if line.starts_with("STATE ") {
                    panic!("upload child finished before the kill — drill degenerate");
                }
            }
            Some(Err(e)) => panic!("read child stdout: {e}"),
            None => panic!("upload child exited before the server ack"),
        }
    };
    assert_eq!(
        server_ack, CHUNK,
        "the server acked exactly the first chunk's span"
    );

    // Kill INSIDE the hold: after the server ack, before the journal write.
    std::thread::sleep(Duration::from_millis(500));
    c1.kill()
        .expect("SIGKILL between server ack and journal write");
    let _ = c1.wait();

    // Journal truth at kill: NOTHING confirmed, job mid-flight. This is the
    // proof the kill landed in the named window, not before it.
    {
        let j = ium_core::SessionJournal::open(&journal).unwrap();
        let jobs = j.list_jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs[0].state,
            ium_core::JobState::Running,
            "the job was mid-transfer"
        );
        let chunks = j.load_chunks(&jobs[0].id).unwrap();
        let confirmed: u64 = chunks.iter().map(|c| c.bytes_confirmed).sum();
        assert_eq!(
            confirmed, 0,
            "the acked bytes must NOT be in the journal — that is the window"
        );
    }

    // -- child 2: a fresh process, a fresh adapter (EMPTY session map) -------
    let out2 = Command::new(exe)
        .args(["resume", "--journal", journal_s])
        .env_clear()
        .envs(tus_env())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .expect("spawn resume child");
    assert!(
        out2.status.success(),
        "resume child failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    let server_offset: u64 = stdout2
        .lines()
        .find_map(|l| l.strip_prefix("SERVER_OFFSET "))
        .expect("SERVER_OFFSET line")
        .parse()
        .expect("SERVER_OFFSET <u64>");
    let wire: u64 = stdout2
        .lines()
        .find_map(|l| l.strip_prefix("WIRE "))
        .expect("WIRE line")
        .parse()
        .expect("WIRE <u64>");
    let wasted: u64 = stdout2
        .lines()
        .find_map(|l| l.strip_prefix("WASTED "))
        .expect("WASTED line — the engine's waste accounting (#30)")
        .parse()
        .expect("WASTED <u64>");
    assert!(
        stdout2.lines().any(|l| l == "DONE done"),
        "honest Done after the read-only HEAD verify, stdout was:\n{stdout2}"
    );

    // The issue's assertions.
    assert_eq!(
        server_offset, CHUNK,
        "the server held exactly the acked chunk — kill-after-ack proven, \
         and SERVER_OFFSET >= last journaled CONFIRMED (0) holds a fortiori"
    );
    // Byte-level zero re-upload (stronger than S3's part-level): the resume
    // process pushed exactly the bytes the server lacked. The rejected 409
    // attempt's flush is measured waste (#30) — subtracted, not slack-bounded.
    let missing = SIZE - server_offset;
    assert!(
        wasted <= wire,
        "waste ({wasted}) cannot exceed wire movement ({wire})"
    );
    assert!(
        wire >= missing,
        "resume moved {wire} of {missing} missing bytes — bytes skipped"
    );
    assert_eq!(
        wire - wasted,
        missing,
        "zero re-upload must be EXACT once the rejected attempt's flush is \
         accounted: every byte the server lacked crossed once, every wasted \
         byte is accounted (wasted={wasted})"
    );
}
