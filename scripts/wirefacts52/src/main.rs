//! Wire-facts rehearsal for issue #52 (FEC lane, RFC 6330 RaptorQ).
//!
//! Runs LIVE on the sandbox before any implementation — the ium-loop rule:
//! no design note claims a fact the sandbox has not executed. Prints one
//! fact per block with W-numbers the design note and issue comment cite.
//!
//! raptorq 1.8.1 API facts learned at compile time (source-read, pinned here):
//!   * Encoder::new(data, config) / Encoder::with_defaults(data, mtu)
//!   * Encoder::get_encoded_packets(repair_per_block) = source + repair PER BLOCK
//!   * SourceBlockEncoder::new2(id, &config, data)  — data.len() % symbol_size == 0 (asserted)
//!   * SourceBlockEncoder::source_packets() = K packets (ESI 0..K)
//!   * SourceBlockEncoder::repair_packets(start_esi, count)
//!   * SourceBlockDecoder::new2(id, &config, block_length)  — K = ceil(block_length / symbol_size)
//!   * SourceBlockDecoder::decode(IntoIterator<Item = EncodingPacket>) -> Option<Vec<u8>>
//!   * EncodingPacket::serialize() = [u8;4 PayloadId] ++ symbol bytes (4 + symbol_size)
//!   * EncodingPacket::deserialize(&[u8]) INFALLIBLE (panics on <4 B input)
//!   * ObjectTransmissionInformation::serialize() = [u8; 12], deserialize INFALLIBLE

use raptorq::{
    EncodingPacket, Encoder, ObjectTransmissionInformation, SourceBlockDecoder,
    SourceBlockEncoder,
};

/// Deterministic xorshift PRNG (no rand dep — the rehearsal must reproduce).
struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn mk_source(len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    let mut x: u32 = 0x9E3779B9;
    for _ in 0..len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        v.push((x >> 24) as u8);
    }
    v
}

fn block_config(symbol_size: u16) -> ObjectTransmissionInformation {
    // The exact config the crate's own deprecated block constructors build.
    ObjectTransmissionInformation::new(0, symbol_size, 0, 1, 1)
}

fn erase_seeded(packets: &[EncodingPacket], drop: usize, seed: u64) -> Vec<EncodingPacket> {
    let mut rng = XorShift(seed);
    let mut idx: Vec<usize> = (0..packets.len()).collect();
    for i in 0..drop.min(idx.len()) {
        let j = i + rng.below(idx.len() - i);
        idx.swap(i, j);
    }
    idx[drop.min(idx.len())..]
        .iter()
        .map(|&i| packets[i].clone())
        .collect()
}

fn main() {
    println!("=== W1: crate identity ===");
    println!(
        "W1 raptorq 1.8.1 (registry source read); packet wire = 4 + symbol_size bytes; OTI wire = 12 bytes"
    );

    println!("=== W2: block-level exact-K control (the fec lane's geometry) ===");
    let sym: u16 = 1024;
    let config = block_config(sym);
    for (k_target, real_len) in [(64usize, 64usize * 1024), (64, 60 * 1024), (32, 15 * 1024)] {
        let padded = real_len + (sym as usize - real_len % sym as usize) % sym as usize;
        let data = mk_source(real_len);
        let mut padded_data = data.clone();
        padded_data.resize(padded, 0);
        let enc = SourceBlockEncoder::new2(0, &config, &padded_data);
        let source = enc.source_packets();
        let repairs = enc.repair_packets(k_target as u32, 16);
        println!(
            "W2 real_len={real_len} padded={padded} K(source)={} R(repair)={} sym={}",
            source.len(),
            repairs.len(),
            source[0].data().len()
        );
        let esi_first = source[0].payload_id().encoding_symbol_id();
        let esi_last_repair = repairs.last().unwrap().payload_id().encoding_symbol_id();
        println!("W2   ESI source[{esi_first}..={}], repair continues to ESI {esi_last_repair}", source.len());
    }

    println!("=== W3: K-of-(K+R) erasure decode, byte-exact, seeded ===");
    let real_len = 64 * 1024;
    let sym = 1024u16;
    let config = block_config(sym);
    let mut data = mk_source(real_len);
    let padded = real_len; // divisible
    data.resize(padded, 0);
    let enc = SourceBlockEncoder::new2(0, &config, &data);
    let mut packets = enc.source_packets();
    packets.extend(enc.repair_packets(64, 16)); // K=64, R=16
    for drop in [1usize, 8, 16] {
        let kept = erase_seeded(&packets, drop, 42);
        let mut dec = SourceBlockDecoder::new2(0, &config, padded as u64);
        let r = dec.decode(kept);
        println!(
            "W3 dropped {drop} of {} -> rebuilt exact: {:?}",
            packets.len(),
            r.map(|d| d == data)
        );
    }

    println!("=== W4: the honesty boundaries ===");
    // below K: repair cannot invent data
    let kept: Vec<EncodingPacket> = packets[..3].to_vec();
    let mut dec = SourceBlockDecoder::new2(0, &config, padded as u64);
    println!("W4 3 packets (<< K=64) -> {:?}", dec.decode(kept).is_some());
    // exactly K source (zero parity used): still exact
    let kept: Vec<EncodingPacket> = packets[..64].to_vec();
    let mut dec = SourceBlockDecoder::new2(0, &config, padded as u64);
    println!("W4 exactly K=64 source, 0 repair -> {:?} (source_packets suffice)", dec.decode(kept).map(|d| d == data));
    // K-1 of K+R: one short — must fail
    let kept = erase_seeded(&packets, 17, 42);
    let mut dec = SourceBlockDecoder::new2(0, &config, padded as u64);
    println!("W4 dropped 17 of 80 (kept 63 < K) -> {:?}", dec.decode(kept).is_some());

    println!("=== W5: wire framing round-trip (what actually crosses the lane) ===");
    let ser = packets[0].serialize();
    println!("W5 packet serialize len={} (4 + sym={})", ser.len(), 4 + sym as usize);
    let back = EncodingPacket::deserialize(&ser);
    println!(
        "W5 deserialize OK: esi={} block={} data_len={}",
        back.payload_id().encoding_symbol_id(),
        back.payload_id().source_block_number(),
        back.data().len()
    );

    println!("=== W6: determinism — same input twice ===");
    let enc2 = SourceBlockEncoder::new2(0, &config, &data);
    let mut p2 = enc2.source_packets();
    p2.extend(enc2.repair_packets(64, 16));
    let same = packets
        .iter()
        .zip(p2.iter())
        .all(|(a, b)| a.serialize() == b.serialize());
    println!("W6 byte-identical packets across encoders: {same}");

    println!("=== W7: overhead accounting at real geometry ===");
    for (g_len, sym) in [(1_048_576usize, 16_384u16), (262_144, 4_096), (65_536, 1_024)] {
        let cfg = block_config(sym);
        let padded = g_len + (sym as usize - g_len % sym as usize) % sym as usize;
        let mut d = mk_source(g_len);
        d.resize(padded, 0);
        let e = SourceBlockEncoder::new2(0, &cfg, &d);
        let k = e.source_packets().len();
        let r = (k as f64 * 0.25).ceil() as u32; // 25% overhead
        let mut all = e.source_packets();
        all.extend(e.repair_packets(k as u32, r));
        let wire: usize = all.iter().map(|p| p.serialize().len()).sum();
        println!(
            "W7 G={g_len} sym={sym}: K={k} R={r} wire={wire} ratio={:.4} (ideal {:.4})",
            wire as f64 / g_len as f64,
            (k as f64 + r as f64) / k as f64
        );
    }

    println!("=== W8: symbolization CPU cost (the tax FEC pays per group) ===");
    let cfg = block_config(16_384);
    let mut d = mk_source(1_048_576);
    d.resize(1_048_576, 0);
    let t0 = std::time::Instant::now();
    let e = SourceBlockEncoder::new2(0, &cfg, &d);
    let mut _p = e.source_packets();
    _p.extend(e.repair_packets(64, 16));
    println!("W8 symbolize 1 MiB + R=16: {:?}", t0.elapsed());
    let t1 = std::time::Instant::now();
    let kept = erase_seeded(&_p, 16, 42);
    let mut dec = SourceBlockDecoder::new2(0, &cfg, 1_048_576);
    let _ = dec.decode(kept);
    println!("W8 decode 1 MiB after R=16 erasures: {:?}", t1.elapsed());

    println!("=== W9: multi-group relay shape — independent groups, erase per group ===");
    let groups: Vec<Vec<u8>> = (0..4).map(|g| mk_source(262_144 + g * 1024)).collect();
    let mut ok = 0;
    for (g_idx, g) in groups.iter().enumerate() {
        let sym = 4096u16;
        let cfg = block_config(sym);
        let padded = g.len() + (sym as usize - g.len() % sym as usize) % sym as usize;
        let mut gd = g.clone();
        gd.resize(padded, 0);
        let e = SourceBlockEncoder::new2(g_idx as u8, &cfg, &gd);
        let k = e.source_packets().len();
        let mut packets = e.source_packets();
        packets.extend(e.repair_packets(k as u32, 16));
        let kept = erase_seeded(&packets, 8, 100 + g_idx as u64);
        let mut dec = SourceBlockDecoder::new2(g_idx as u8, &cfg, padded as u64);
        if dec.decode(kept).map(|x| x[..g.len()] == **g) == Some(true) {
            ok += 1;
        }
    }
    println!("W9 4 groups (K varies with tail), 8 erasures each (R=16): {ok}/4 rebuilt byte-exact");

    println!("=== W10: decode result trimming (padded tail -> true length) ===");
    let real = 60 * 1024;
    let sym = 1024u16;
    let cfg = block_config(sym);
    let mut d = mk_source(real);
    d.resize(64 * 1024, 0); // pad to symbol boundary
    let e = SourceBlockEncoder::new2(0, &cfg, &d);
    let mut packets = e.source_packets();
    packets.extend(e.repair_packets(64, 4));
    let mut dec = SourceBlockDecoder::new2(0, &cfg, 64 * 1024);
    let r = dec.decode(packets).unwrap();
    println!(
        "W10 decoded len={} true_len={pad_true}; first {real} bytes exact: {}",
        r.len(),
        r[..real] == mk_source(real),
        pad_true = 64 * 1024
    );

    println!("=== W11: PINNED VECTOR (deterministic capture for the unit test) ===");
    let cfg = block_config(1024);
    let d = mk_source(8192);
    let e = SourceBlockEncoder::new2(7, &cfg, &d);
    let p = e.source_packets();
    let r1 = e.repair_packets(8, 1);
    let v0 = p[0].serialize();
    let v8 = r1[0].serialize();
    println!(
        "W11 src[0] first 12 bytes: {:02x?}",
        &v0[..12.min(v0.len())]
    );
    println!(
        "W11 repair ESI=8 first 12 bytes: {:02x?}",
        &v8[..12.min(v8.len())]
    );
    println!("W11 K={} packet0_wire_len={}", p.len(), v0.len());

    pin_module_shape();
    println!("=== DONE ===");
}

// W12 addendum: pin the vector with the MODULE-shaped call (repair start = 0).
#[allow(dead_code)]
fn pin_module_shape() {
    let cfg = ObjectTransmissionInformation::new(0, 1024, 0, 1, 1);
    let d = mk_source(8192);
    let e = SourceBlockEncoder::new2(7, &cfg, &d);
    let p = e.source_packets();
    let r = e.repair_packets(0, 1);
    let v0 = p[0].serialize();
    let vr = r[0].serialize();
    println!("W12 K={} repair_esi={} v0={:02x?} vr={:02x?}", p.len(), r[0].payload_id().encoding_symbol_id(), &v0[..12], &vr[..12]);
}
