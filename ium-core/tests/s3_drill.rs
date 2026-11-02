//! The kill-9 drill, parent side (issue #9) — NIGHTLY, Linux, env-gated.
//!
//! A >100 MiB multipart upload runs in a REAL child process (real manager,
//! real S3 adapter, real journal). The parent waits for the first completed
//! part to appear in the child's journal, SIGKILLs the child mid-body, then
//! runs a FRESH child on the same journal and asserts:
//!
//! * `DONE done` — honest Done, declared only after the ADR-010 verify gate;
//! * the wire bytes moved by the RESUME process are exactly the missing
//!   parts — zero re-upload of the completed part (wire-byte counter);
//! * `WIRE < file size` (the acceptance criterion).
//!
//! Deterministic by construction: the child's adapter takes tickets
//! (atomic arrivals) so exactly ONE upload call ever proceeds; the rest
//! wedge at entry with zero bytes on the wire and zero confirms in the
//! journal. The kill lands in a stable, fully specified state.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

const SIZE: u64 = 112 * 1024 * 1024; // >100 MiB per the AC
const WORKERS: u32 = 4;
/// 112 MiB / 4 — the even split's part size (and the ONE part that completes
/// before the wedge freezes everything else).
const FIRST_PART: u64 = SIZE / WORKERS as u64;

fn live_endpoint() -> Option<String> {
    match std::env::var("IUM_S3_ENDPOINT") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("skipped: IUM_S3_ENDPOINT not set (harness provides the server)");
            None
        }
    }
}

fn write_zeros(path: &std::path::Path, size: u64) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    let block = vec![0u8; 1024 * 1024];
    let mut left = size;
    while left > 0 {
        let n = block.len().min(left as usize);
        f.write_all(&block[..n]).unwrap();
        left -= n as u64;
    }
    f.sync_all().unwrap();
}

fn s3_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| k.starts_with("IUM_S3_"))
        .collect()
}

#[test]
fn kill9_midbody_then_resume_zero_reupload_and_honest_done() {
    let Some(_endpoint) = live_endpoint() else {
        return;
    };
    let exe = env!("CARGO_BIN_EXE_s3_drill_child");
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("drill112.bin");
    write_zeros(&src, SIZE);
    let journal = dir.path().join("drill.journal.db");
    let journal_s = journal.to_str().unwrap();
    let src_s = src.to_str().unwrap();

    // -- child 1: real upload, frozen after exactly one completed part ------
    let mut c1 = Command::new(exe)
        .args([
            "upload",
            "--journal",
            journal_s,
            "--file",
            src_s,
            "--allow-calls",
            "1",
        ])
        .env_clear()
        .envs(s3_env())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn upload child");
    let mut out1 = BufReader::new(c1.stdout.take().unwrap()).lines();

    let mut confirmed: u64;
    loop {
        match out1.next() {
            Some(Ok(line)) => {
                if let Some(v) = line.strip_prefix("CONFIRMED ") {
                    confirmed = v.parse().expect("CONFIRMED <u64>");
                    if confirmed >= FIRST_PART {
                        // One part is confirmed in the journal and the ticket
                        // gate froze the rest — the reading cannot move again.
                        break;
                    }
                }
                if line.starts_with("STATE done") {
                    panic!("upload child finished before the kill — drill degenerate");
                }
            }
            Some(Err(e)) => panic!("read child stdout: {e}"),
            None => panic!("upload child exited before any part was confirmed"),
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(300)); // let journals settle
    c1.kill().expect("SIGKILL the child mid-body");
    let _ = c1.wait();
    assert_eq!(
        confirmed, FIRST_PART,
        "exactly the first part confirmed before the kill"
    );

    // -- child 2: a fresh process recovers from the journal ----------------
    let out2 = Command::new(exe)
        .args(["resume", "--journal", journal_s])
        .env_clear()
        .envs(s3_env())
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
    let wire: u64 = stdout2
        .lines()
        .find_map(|l| l.strip_prefix("WIRE "))
        .expect("WIRE line")
        .parse()
        .expect("WIRE <u64>");
    assert!(
        stdout2.lines().any(|l| l == "DONE done"),
        "honest Done after the ADR-010 verify gate, stdout was:\n{stdout2}"
    );

    // The acceptance criterion, and the stronger invariant behind it.
    assert!(
        wire < SIZE,
        "resume moved {wire} of {SIZE} bytes — re-upload happened"
    );
    assert_eq!(
        wire,
        SIZE - FIRST_PART,
        "exactly the missing parts crossed the wire: the completed part was NOT re-uploaded"
    );
}
