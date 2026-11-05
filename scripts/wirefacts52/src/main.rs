//! Wire-facts rehearsal for issue #52 (FEC lane, RFC 6330 RaptorQ).
//!
//! Runs LIVE on the sandbox before any implementation — the ium-loop rule:
//! no design note claims a fact the sandbox has not executed. Prints one
//! fact per block with W-numbers the design note and issue comment cite.

use raptorq::{Decoder, EncodingPacket, Encoder, ObjectTransmissionInformation};

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

fn main() {
    println!("=== W1: crate identity + API smoke ===");
    let src = mk_source(1_000_000);
    let encoder = Encoder::new(&src, 1024);
    let oti = encoder.get_config();
    println!(
        "W1 Encoder::new(data, max_packet_size=1024) ok; OTI transfer_length={} (raw bytes: {:?})",
        oti.transfer_length(),
        oti.serialize()
    );
    println!("W1 OTI serialize() len = {}", oti.serialize().len());

    println!("=== W2: get_encoded_packets semantics (K + repair or repair only?) ===");
    let packets = encoder.get_encoded_packets(10);
    println!("W2 get_encoded_packets(10) on 1_000_000 B / 1024 B MTU returned {} packets", packets.len());
    // Try to observe the symbol payload length from a serialized packet.
    let ser = packets[0].clone().serialize();
    println!("W2 packet serialize() len = {} (payload 1024? overhead = {})", ser.len(), ser.len() as i64 - 1024);

    println!("=== W3: round-trip with ZERO loss ===");
    let mut decoder = Decoder::new(oti);
    let decoded = decoder.decode(packets.clone());
    match &decoded {
        Some(d) => println!(
            "W3 zero-loss decode OK: len={} exact={}",
            d.len(),
            d == src
        ),
        None => println!("W3 zero-loss decode FAILED — decoder could not rebuild"),
    }

    println!("=== W4: K-of-(K+R) erasure decode (seeded) ===");
    // packets.len() = K + repair (or repair-only, per W2). Erase R of them.
    for drop in [1usize, 3, 10] {
        let mut rng = XorShift(42);
        let mut idx: Vec<usize> = (0..packets.len()).collect();
        // partial Fisher-Yates: pick `drop` distinct indices
        for i in 0..drop {
            let j = i + rng.below(idx.len() - i);
            idx.swap(i, j);
        }
        let kept: Vec<usize> = idx[drop..].to_vec();
        let subset: Vec<EncodingPacket> = kept.iter().map(|&i| packets[i].clone()).collect();
        let mut d = Decoder::new(oti);
        let r = d.decode(subset);
        println!("W4 dropped {drop} of {} kept {} -> decode {:?}", packets.len(), kept.len(), r.is_some());
    }

    println!("=== W5: below-K decode must fail (honesty of the repair bound) ===");
    let subset: Vec<EncodingPacket> = (0..3).map(|i| packets[i].clone()).collect();
    let mut d = Decoder::new(oti);
    let r = d.decode(subset);
    println!("W5 3 packets (<< K) -> decode {:?}", r.is_some());

    println!("=== W6: exact-K decode (the parity-free boundary) ===");
    let subset: Vec<EncodingPacket> = (0..packets.len() - 10).map(|i| packets[i].clone()).collect();
    let mut d = Decoder::new(oti);
    let r = d.decode(subset);
    println!(
        "W6 kept K'={} (K+R-10) -> decode {:?}",
        packets.len() - 10,
        r.map(|d| d == src)
    );

    println!("=== W7: determinism — same input twice, byte-equal packets? ===");
    let again = Encoder::new(&src, 1024).get_encoded_packets(10);
    let same = again
        .iter()
        .zip(packets.iter())
        .all(|(a, b)| a.clone().serialize() == b.clone().serialize());
    println!("W7 deterministic across encoders: {}", same);

    println!("=== W8: overhead accounting at real geometry (K=64, ~1 MiB group) ===");
    // Group G = 1 MiB, want K ≈ 64 source symbols of 16 KiB.
    for (g_len, sym) in [(1_048_576usize, 16_384u16), (262_144, 4_096), (65_536, 1_024)] {
        let e = Encoder::new(&mk_source(g_len), sym);
        let k = e.get_encoded_packets(0).len();
        let cfg = e.get_config();
        println!(
            "W8 G={g_len} sym={sym}: K={k} (element_size={}?) oti_bytes={}",
            cfg.element_size(),
            cfg.serialize().len()
        );
        let r = 16u16; // 25% overhead
        let all = e.get_encoded_packets(r);
        let wire: usize = all.iter().map(|p| p.clone().serialize().len()).sum();
        println!(
            "W8   +R={r}: {} packets, wire={wire} B, ratio={:.4} ((K+R)/K ideal={:.4})",
            all.len(),
            wire as f64 / g_len as f64,
            (k as f64 + r as f64) / k as f64
        );
    }

    println!("=== W9: symbolization COST (CPU per group — the tax FEC pays) ===");
    let t0 = std::time::Instant::now();
    let e = Encoder::new(&mk_source(1_048_576), 16_384);
    let _ = e.get_encoded_packets(16);
    println!("W9 symbolize 1 MiB + 16 repair: {:?}", t0.elapsed());

    println!("=== W10: sub-blocking — small MTU, multi-block object decodes across erasures ===");
    let big = mk_source(4_000_000);
    let e = Encoder::new(&big, 256); // tiny MTU -> many blocks/sub-blocks
    let all = e.get_encoded_packets(400);
    let mut rng = XorShift(7);
    let mut idx: Vec<usize> = (0..all.len()).collect();
    for i in 0..400 {
        let j = i + rng.below(idx.len() - i);
        idx.swap(i, j);
    }
    let subset: Vec<EncodingPacket> = idx[400..].iter().map(|&i| all[i].clone()).collect();
    let mut d = Decoder::new(e.get_config());
    let r = d.decode(subset);
    println!(
        "W10 4 MB / 256 B MTU: {} packets, dropped 400 -> decode {:?}",
        all.len(),
        r.map(|d| d == big)
    );

    println!("=== W11: deserialize round-trip (wire framing must be self-describing) ===");
    let ser = packets[0].clone().serialize();
    match EncodingPacket::deserialize(&ser) {
        Some(p) => println!(
            "W11 EncodingPacket::deserialize OK (esi={}, payload_len={})",
            p.esi(),
            p.symbol().len()
        ),
        None => println!("W11 deserialize FAILED"),
    }
    let oti_ser = oti.serialize();
    match ObjectTransmissionInformation::deserialize(&oti_ser) {
        Some(o) => println!("W11 OTI::deserialize OK transfer_length={}", o.transfer_length()),
        None => println!("W11 OTI deserialize FAILED"),
    }

    println!("=== W12: multi-group relay decode — independent groups, erase per group ===");
    // The fec lane's real shape: G-sized groups decoded INDEPENDENTLY.
    let groups: Vec<Vec<u8>> = (0..4).map(|g| mk_source(262_144 + g)).collect();
    let mut all_group_packets: Vec<Vec<EncodingPacket>> = Vec::new();
    for g in &groups {
        let e = Encoder::new(g, 4_096);
        all_group_packets.push(e.get_encoded_packets(16));
    }
    let mut ok = 0;
    for (g, packets) in groups.iter().zip(all_group_packets.iter()) {
        let mut rng = XorShift(100 + g.len() as u64);
        let mut idx: Vec<usize> = (0..packets.len()).collect();
        for i in 0..8 {
            let j = i + rng.below(idx.len() - i);
            idx.swap(i, j);
        }
        let subset: Vec<EncodingPacket> = idx[8..].iter().map(|&i| packets[i].clone()).collect();
        let mut d = Decoder::new(Encoder::new(g, 4_096).get_config());
        if d.decode(subset).map(|x| x == *g) == Some(true) {
            ok += 1;
        }
    }
    println!("W12 4 groups, 8 erasures each (R=16): {ok}/4 rebuilt byte-exact");

    println!("=== DONE ===");
}
