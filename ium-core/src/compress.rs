//! The entropy gate + zstd archive primitives (issue #53): Cause-D byte
//! reduction — when the bearer itself is 3 KB/s the only software lever is
//! FEWER BYTES, and the gate must say so in seconds when the payload will
//! not compress.
//!
//! Wire facts (rehearsed live, `scripts/wirefacts53/`): the gate
//! discriminates at the 1.15 bar with fast level 3 on head/mid/tail 64 KiB
//! samples (text 6.05× ACCEPT vs video-like 1.00× DECLINE, W2); a corrupted
//! archive fails decode loudly when the encoder set `include_checksum` (W4);
//! `statvfs` works unprivileged for the #34 disk check (W5).
//!
//! Everything here is deterministic for a fixed input: the gate's ratio is
//! reproducible (the tests pin it), and the archive is a plain file whose
//! resume properties are the wrapped lane's own (D2 of the design note).

use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The honest bar: below a 1.15× projected ratio the wire saving is not
/// worth the CPU (the issue's number — video/mp4/jpg live here).
pub const GATE_BAR: f64 = 1.15;

/// Sample size: head / middle / tail, the REAL bytes (64 KiB each).
pub const SAMPLE_BYTES: u64 = 64 * 1024;

/// The gate's fast level (seconds, not minutes — the decision must be cheap).
const GATE_LEVEL: i32 = 3;

/// The gate's verdict, with the ratio it measured — honesty means the
/// reported number IS the recomputed one (the tests pin that).
#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    /// Projected ratio ≥ [`GATE_BAR`] — compress.
    Accept {
        /// The measured projected ratio (raw / packed over the samples).
        projected_ratio: f64,
    },
    /// Below the bar (or nothing to gain) — decline, journal why.
    Decline {
        /// The measured projected ratio (raw / packed over the samples).
        projected_ratio: f64,
        /// The decline reason ("incompressible").
        why: &'static str,
    },
}

impl GateDecision {
    /// Would the gate compress?
    pub fn accepted(&self) -> bool {
        matches!(self, GateDecision::Accept { .. })
    }

    /// The measured projected ratio (raw / packed over the samples).
    pub fn projected_ratio(&self) -> f64 {
        match self {
            GateDecision::Accept { projected_ratio }
            | GateDecision::Decline {
                projected_ratio, ..
            } => *projected_ratio,
        }
    }

    /// The decline reason ("incompressible"), if declined.
    pub fn why(&self) -> Option<&'static str> {
        match self {
            GateDecision::Accept { .. } => None,
            GateDecision::Decline { why, .. } => Some(why),
        }
    }
}

/// The sampling law on bytes: head / middle / tail 64 KiB; anything ≤ 3
/// samples big is sampled WHOLE (small files cannot afford blind spots).
pub fn sample_bytes(data: &[u8]) -> Vec<Vec<u8>> {
    let s = SAMPLE_BYTES as usize;
    if (data.len() as u64) <= 3 * SAMPLE_BYTES {
        return vec![data.to_vec()];
    }
    let n = data.len();
    vec![
        data[..s].to_vec(),
        data[n / 2 - s / 2..n / 2 + s / 2].to_vec(),
        data[n - s..].to_vec(),
    ]
}

/// The sampling law on a file: three bounded seek-reads (never buffers the
/// whole source — the 100 MiB nightly file pays 192 KiB of sampling I/O).
pub fn sample_source(path: &Path, size: u64) -> std::io::Result<Vec<Vec<u8>>> {
    let mut f = std::fs::File::open(path)?;
    let s = SAMPLE_BYTES;
    if size <= 3 * s {
        let mut v = Vec::new();
        f.read_to_end(&mut v)?;
        return Ok(vec![v]);
    }
    let mut read_at = |off: u64| -> std::io::Result<Vec<u8>> {
        f.seek(SeekFrom::Start(off))?;
        let mut buf = vec![0u8; s as usize];
        f.read_exact(&mut buf)?;
        Ok(buf)
    };
    Ok(vec![
        read_at(0)?,
        read_at(size / 2 - s / 2)?,
        read_at(size - s)?,
    ])
}

/// The gate: compress each sample at the fast level, project the ratio,
/// compare against [`GATE_BAR`]. An empty source DECLINES (a zstd frame of
/// nothing is BIGGER than nothing — nothing to gain).
pub fn entropy_gate(samples: &[Vec<u8>]) -> GateDecision {
    let raw: usize = samples.iter().map(|x| x.len()).sum();
    if raw == 0 {
        return GateDecision::Decline {
            projected_ratio: 0.0,
            why: "incompressible",
        };
    }
    let packed: usize = samples
        .iter()
        .map(|x| {
            zstd::stream::encode_all(&x[..], GATE_LEVEL)
                .unwrap_or_else(|_| x.clone())
                .len()
        })
        .sum();
    let ratio = raw as f64 / packed as f64;
    if ratio < GATE_BAR {
        GateDecision::Decline {
            projected_ratio: ratio,
            why: "incompressible",
        }
    } else {
        GateDecision::Accept {
            projected_ratio: ratio,
        }
    }
}

/// A reader that hashes what it reads — one pass computes the source's
/// sha256 WHILE the archive streams out (W6's io::copy shape).
struct HashingReader<R: Read> {
    inner: R,
    hasher: Sha256,
    count: u64,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }
}

/// Compress the whole source to `zst` (checksummed frame), returning
/// `(archive_size, source_sha256_hex)` from ONE pass over the source.
/// Bounded memory: streaming encode, file to file.
pub fn compress_source(src: &Path, zst: &Path, level: i32) -> std::io::Result<(u64, String)> {
    let file = std::fs::File::open(src)?;
    let mut reader = HashingReader {
        inner: file,
        hasher: Sha256::new(),
        count: 0,
    };
    let mut enc = zstd::stream::Encoder::new(std::fs::File::create(zst)?, level)?;
    enc.include_checksum(true)?;
    std::io::copy(&mut reader, &mut enc)?;
    let archive = enc.finish()?;
    let hex = hex::encode(reader.hasher.finalize());
    Ok((archive.metadata()?.len(), hex))
}

/// sha256 of a file, streamed (64 KiB chunks) — the archive's own hash and
/// any re-hash the resume path needs.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// The three-in-one integrity check (D4): decode the archive (checksum-
/// validated by the zstd frame — W4), sha256 the DECODED bytes, compare with
/// the journaled source hash and size. `Ok(false)` = decoded but wrong;
/// `Err` = the frame refused to decode (corruption — loud, never silent).
pub fn verify_archive(
    zst: &Path,
    expect_sha256_hex: &str,
    expect_size: u64,
) -> std::io::Result<bool> {
    let mut dec = zstd::stream::Decoder::new(std::fs::File::open(zst)?)?;
    let mut hasher = Sha256::new();
    let mut count: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = dec.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        count += n as u64;
    }
    Ok(count == expect_size && hex::encode(hasher.finalize()) == expect_sha256_hex)
}
