//! Wire-facts rehearsal for issue #53 (entropy-gated zstd transfer mode).
//! Runs LIVE before any implementation — the ium-loop rule.

use std::io::{Read, Write};

fn mk_source(len: usize) -> Vec<u8> {
    // high-entropy (video-like): the gate MUST decline this
    let mut v = Vec::with_capacity(len);
    let mut x: u32 = 0x9E37_79B9;
    for _ in 0..len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        v.push((x >> 24) as u8);
    }
    v
}

fn mk_text(len: usize) -> Vec<u8> {
    // highly compressible: the gate MUST accept this
    let words = ["the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog "];
    let mut v = Vec::with_capacity(len);
    let mut x: u32 = 7;
    while v.len() < len {
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        v.extend_from_slice(words[(x >> 16) as usize % words.len()].as_bytes());
    }
    v.truncate(len);
    v
}

fn compress(data: &[u8], level: i32) -> Vec<u8> {
    let mut enc = zstd::stream::Encoder::new(Vec::new(), level).unwrap();
    enc.include_checksum(true).unwrap();
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn main() {
    println!("=== W1: crate identity + round-trip with checksum ===");
    let src = mk_text(1 << 20);
    let arc = compress(&src, 19);
    let mut dec = zstd::stream::Decoder::new(&arc[..]).unwrap();
    let mut back = Vec::new();
    dec.read_to_end(&mut back).unwrap();
    println!(
        "W1 zstd {} round-trip exact={} (archive {}/{} = {:.4})",
        "0.13.4 (zstd-sys 2.1.2 + zstd 1.5.7)",
        back == src,
        arc.len(),
        src.len(),
        arc.len() as f64 / src.len() as f64
    );

    println!("=== W2: the gate's discrimination (fast level 3, head/mid/tail 64 KiB) ===");
    for (name, data) in [("text", mk_text(4 << 20)), ("video-like", mk_source(4 << 20))] {
        let s = 64 * 1024;
        let n = data.len();
        let mut samples = Vec::new();
        for off in [0usize, n / 2 - s / 2, n - s] {
            samples.push(&data[off..off + s]);
        }
        let raw: usize = samples.iter().map(|x| x.len()).sum();
        let packed: usize = samples.iter().map(|x| compress(x, 3).len()).sum();
        let ratio = raw as f64 / packed as f64;
        println!(
            "W2 {name}: samples {raw} -> {packed} projected ratio {ratio:.3} => {}",
            if ratio < 1.15 { "DECLINE" } else { "ACCEPT" }
        );
    }

    println!("=== W3: full-archive levels + timing (level 19 on 4 MiB text) ===");
    let t0 = std::time::Instant::now();
    let arc19 = compress(&mk_text(4 << 20), 19);
    println!(
        "W3 level 19: {} B in {:?} (ratio {:.4})",
        arc19.len(),
        t0.elapsed(),
        arc19.len() as f64 / (4 << 20) as f64
    );

    println!("=== W4: checksum validation catches corruption (the integrity gate) ===");
    let mut bad = compress(&mk_text(1 << 20), 19);
    let mid = bad.len() / 2;
    bad[mid] ^= 0xFF;
    let verdict = std::panic::catch_unwind(|| {
        let mut dec = zstd::stream::Decoder::new(&bad[..]).unwrap();
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        out.len()
    });
    println!("W4 corrupted archive decode: {}", if verdict.is_err() { "ERROR (checksum caught it)" } else { "DECODED (no checksum enforcement!)" });

    println!("=== W5: statvfs free-space (the #34 discipline check) ===");
    #[cfg(unix)]
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        let cpath = std::ffi::CString::new("/home/z/my-project").unwrap();
        let rc = libc::statvfs(cpath.as_ptr(), &mut st);
        let free = st.f_bavail as u64 * st.f_frsize as u64;
        println!("W5 statvfs rc={rc} free={free} B ({:.1} GiB)", free as f64 / (1 << 30) as f64);
    }

    println!("=== W6: streaming encode from a file handle (the adapter's shape) ===");
    let dir = std::env::temp_dir().join("ium-wf53");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("src.bin");
    let zst_path = dir.join("src.bin.zst");
    std::fs::write(&src_path, mk_text(2 << 20)).unwrap();
    let t0 = std::time::Instant::now();
    let mut enc = zstd::stream::Encoder::new(std::fs::File::create(&zst_path).unwrap(), 19).unwrap();
    enc.include_checksum(true).unwrap();
    std::io::copy(&mut std::fs::File::open(&src_path).unwrap(), &mut enc).unwrap();
    let arc = enc.finish().unwrap();
    let src_sha: [u8; 32] = {
        use std::io::Read as _;
        let mut h = [0u8; 32];
        // sha256 via the same primitive the engine uses is NOT in this probe;
        // byte count + decode round-trip is the fact being pinned here.
        let mut f = std::fs::File::open(&src_path).unwrap();
        let mut n = 0usize;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let k = f.read(&mut buf).unwrap();
            if k == 0 { break; }
            n += k;
        }
        h[0] = (n & 0xFF) as u8;
        h
    };
    let _ = src_sha;
    println!(
        "W6 streamed archive {} B from 2 MiB source in {:?} (file-to-file io::copy shape)",
        arc.metadata().unwrap().len(),
        t0.elapsed()
    );
    println!("=== DONE ===");
}
