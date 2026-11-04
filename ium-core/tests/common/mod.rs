//! Shared test helpers: the REAL video fixture (issue #17, owner mandate
//! "always test on real data") used by the real-data drill and the
//! slow-connection drill (#18). The fixture is generated at test time by
//! ffmpeg and never committed (ADR-009).

pub mod tus_mock;

use std::io::{Read, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

pub const SAMPLE_WINDOW: usize = 64 * 1024;
pub const DISTINCT_BYTES_REQUIRED: usize = 64;

pub struct Fixture {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
}

pub fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generate the real fixture; `None` = loud skip (no ffmpeg/ffprobe).
pub fn real_video(dir: &Path) -> Option<Fixture> {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!(
            "skipped: ffmpeg/ffprobe not on PATH — the real-data drill refuses to \
             fake it with zeros (owner mandate: always test on real data)"
        );
        return None;
    }
    let out = dir.join("ium-real-fixture.mp4");
    let st = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
            "-t",
            "45",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "4M",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-shortest",
            "-movflags",
            "+faststart",
        ])
        .arg(&out)
        .output()
        .expect("spawn ffmpeg");
    assert!(
        st.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
    let size = out.metadata().expect("fixture stat").len();
    assert!(
        size > 16 * 1024 * 1024,
        "fixture too small to be multi-chunk: {size}"
    );
    assert_real_video(&out, "source");
    assert_not_trivial(&out, size, "source");
    let sha256 = sha256_file(&out);
    Some(Fixture {
        path: out,
        size,
        sha256,
    })
}

/// ffprobe guards: a REAL video (h264 + aac, duration > 0) — pins that the
/// drill can never silently degrade to zeros or random bytes.
pub fn assert_real_video(path: &Path, label: &str) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format"])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(
        out.status.success(),
        "ffprobe failed on {label}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("codec_name=h264") && text.contains("codec_type=video"),
        "{label} is not a real H.264 video (mandate violation)"
    );
    assert!(
        text.contains("codec_name=aac") && text.contains("codec_type=audio"),
        "{label} lost its audio stream"
    );
    let duration = text
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0);
    assert!(
        duration > 0.0,
        "{label} has no duration — not a playable video"
    );
}

/// Entropy guard: a 64 KiB window around the middle must hold > 64 distinct
/// byte values. Zeros (or any single-byte pattern) hold exactly 1 — this is
/// the cheap, std-only tripwire that keeps the drill honest.
pub fn assert_not_trivial(path: &Path, size: u64, label: &str) {
    let mut f = std::fs::File::open(path).expect("open for entropy check");
    let window = SAMPLE_WINDOW.min(size as usize);
    let mid = (size / 2).saturating_sub(window as u64 / 2);
    f.seek(std::io::SeekFrom::Start(mid)).expect("seek");
    let mut buf = vec![0u8; window];
    f.read_exact(&mut buf).expect("read entropy window");
    let distinct = buf.iter().collect::<std::collections::HashSet<_>>().len();
    assert!(
        distinct > DISTINCT_BYTES_REQUIRED,
        "{label} looks trivial ({distinct} distinct byte values in a {window}-byte window) — \
         the real-data drill only accepts real content"
    );
}

pub fn sha256_file(path: &Path) -> String {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(path).expect("open for sha256");
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf).expect("read for sha256");
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    hex(h.finalize())
}

pub fn sha256_bytes(b: &[u8]) -> String {
    hex(Sha256::digest(b))
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Persist received bytes and run the SAME reality guards on them — the
/// destination must hold a playable video, not just the right byte count.
pub fn assert_received_is_real_video(bytes: &[u8], dir: &Path, label: &str) {
    let p = dir.join(format!("{label}-received.mp4"));
    let mut f = std::fs::File::create(&p).expect("write received");
    f.write_all(bytes).expect("write received");
    f.sync_all().expect("sync received");
    assert_real_video(&p, label);
    assert_not_trivial(&p, bytes.len() as u64, label);
}
