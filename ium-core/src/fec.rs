//! Pure FEC layer (issue #52): RFC 6330 RaptorQ geometry, wire framing,
//! symbolization and rebuild — no I/O, no adapter knowledge.
//!
//! The relay contract in one paragraph: the client splits the source into
//! *groups* of `G` bytes (resplit-stable: derived from `source_size` alone),
//! symbolizes each group into `K` source + `R` repair symbols (RaptorQ
//! RFC 6330, crate `raptorq` 1.8.1 — wire facts W1–W12 rehearsed live in
//! `scripts/wirefacts52/`), and ships each group as one framed byte stream
//! over an ordinary resumable lane session (see [`crate::adapter_fec`]).
//! Any `K` of the group's `K+R` packets rebuild the group's bytes exactly
//! ([`rebuild_group`]) — erasure repair with zero round-trips. The decoder
//! side of this module is exactly what an IUM relay calls.
//!
//! Wire facts pinned live (see the issue for the full table): packet wire =
//! 4 B `PayloadId` + symbol bytes; repair ESI =
//! `extended_source_block_symbols(K) + start + i` (RFC 6330 §5.3.4); the
//! encoder is deterministic for a fixed input; a group's bytes must be a
//! symbol-size multiple (zero-padded tail, trimmed at decode by the true
//! length).

use raptorq::{
    EncodingPacket, ObjectTransmissionInformation, SourceBlockDecoder, SourceBlockEncoder,
};

/// Wire header of one framed group stream: `u32 BE true_len` + `u16 BE
/// symbol_size`. Every packet that follows is exactly
/// `4 + symbol_size` bytes, so any byte-prefix of a stream parses up to the
/// last complete packet ([`parse_stream`] refuses torn tails — a crashed
/// PATCH's realign point is a framing error, never silent truncation).
pub const HEADER_BYTES: usize = 6;

/// FEC geometry configuration (the issue's "overhead % configurable").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FecConfig {
    /// Symbol size in bytes — every packet carries one symbol. Must be a
    /// multiple of the RFC alignment (8). Default 16 KiB.
    pub symbol_size: u16,
    /// Repair overhead as a percent of source symbols per group. Default 25
    /// (R = ⌈K/4⌉): any ≤25% per-group erasure rate repairs without
    /// round-trips. 0 is legal (parity-free lane — the ladder measures it).
    pub overhead_pct: u32,
    /// Default source symbols per group: `G = k_default × symbol_size`.
    /// Default 64 → G = 1 MiB.
    pub k_default: u32,
    /// Hard cap on group count: beyond it G GROWS (symbol-aligned) so the
    /// eager session cost stays bounded (4096 POSTs worst case).
    pub max_groups: u32,
}

impl Default for FecConfig {
    fn default() -> Self {
        FecConfig {
            symbol_size: 16 * 1024,
            overhead_pct: 25,
            k_default: 64,
            max_groups: 4096,
        }
    }
}

impl FecConfig {
    /// Source bytes per (default) group: `k_default × symbol_size`.
    pub fn group_bytes(&self) -> u64 {
        self.k_default as u64 * self.symbol_size as u64
    }
}

/// Derived, journaled geometry for one job: group boundaries, per-group K/R,
/// wire sizes. Derived from `source_size` ALONE — a resplit, a resume or a
/// fresh process re-derives the identical geometry (the resplit-stability the
/// issue demands; the chunk pool's piece grid is irrelevant to it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FecGeometry {
    /// The job's source size in bytes.
    pub source_size: u64,
    /// Source bytes per FULL group (symbol-size multiple; the tail group is
    /// smaller). Never exceeds `u32::MAX` (the framing header's true_len is
    /// u32) — [`FecConfig::max_groups`] growth keeps it far below in practice.
    pub group_bytes: u64,
    /// Symbol size in bytes (from the config).
    pub symbol_size: u16,
    /// Repair overhead percent (from the config).
    pub overhead_pct: u32,
    /// Group count — always ≥ 1 (a 0-byte file is ONE empty group).
    pub groups: u32,
}

impl FecGeometry {
    /// Derive the geometry for `source_size` under `config`.
    ///
    /// Infallible: G grows (rounded up to the symbol size) when the default
    /// grid would exceed `max_groups`; a 0-byte file derives one empty group.
    pub fn derive(config: &FecConfig, source_size: u64) -> FecGeometry {
        let sym = config.symbol_size as u64;
        debug_assert!(
            sym > 0 && config.symbol_size.is_multiple_of(8),
            "symbol size must be a positive multiple of the RFC alignment (8)"
        );
        let mut group_bytes = config.group_bytes();
        if source_size > 0 && source_size.div_ceil(group_bytes) > config.max_groups as u64 {
            group_bytes = source_size.div_ceil(config.max_groups as u64).div_ceil(sym) * sym;
        }
        let groups = if source_size == 0 {
            1
        } else {
            source_size.div_ceil(group_bytes) as u32
        };
        FecGeometry {
            source_size,
            group_bytes,
            symbol_size: config.symbol_size,
            overhead_pct: config.overhead_pct,
            groups,
        }
    }

    /// Number of groups (≥ 1).
    pub fn group_count(&self) -> u32 {
        self.groups
    }

    /// TRUE byte length of group `g` (the tail group is shorter; a 0-byte
    /// file's single group is 0). Panics on `g ≥ groups` — a caller bug.
    pub fn group_true_len(&self, g: u32) -> u64 {
        assert!(g < self.groups, "group index out of range");
        let start = g as u64 * self.group_bytes;
        (self.source_size - start).min(self.group_bytes)
    }

    /// Source symbols for group `g`: `⌈true_len / symbol_size⌉` (0 for the
    /// empty group). This is the EXACT K the encoder produces (wire fact W2:
    /// the block encoder asserts `data.len() % symbol_size == 0`, so the
    /// symbolizer zero-pads to `K × symbol_size`).
    pub fn k(&self, g: u32) -> u32 {
        self.group_true_len(g).div_ceil(self.symbol_size as u64) as u32
    }

    /// Repair symbols for group `g`: `⌈K × overhead_pct / 100⌉`.
    pub fn r(&self, g: u32) -> u32 {
        (self.k(g) as u64 * self.overhead_pct as u64).div_ceil(100) as u32
    }

    /// Wire bytes of group `g`'s framed stream: header + (K+R) packets of
    /// `4 + symbol_size` bytes each (wire facts W1/W5/W7).
    pub fn wire_len(&self, g: u32) -> u64 {
        HEADER_BYTES as u64 + (self.k(g) + self.r(g)) as u64 * (4 + self.symbol_size as u64)
    }

    /// Total wire bytes across all groups (the fec lane's honest wire cost).
    pub fn total_wire(&self) -> u64 {
        (0..self.groups).map(|g| self.wire_len(g)).sum()
    }

    /// Measured wire amplification: `total_wire / max(source_size, 1)` —
    /// ≈ (K+R)/K plus the per-packet 4 B framing (wire fact W7: 1.2503 at
    /// the default geometry).
    pub fn overhead_ratio(&self) -> f64 {
        self.total_wire() as f64 / self.source_size.max(1) as f64
    }

    /// The `fec_plan` journal payload (the issue's `fec_plan {groups, K, R,
    /// overhead}` — geometry RULES, journaled inside the probe event's
    /// session token; the per-group session tokens ride beside them).
    pub fn journal_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "v": 1,
            "sym": self.symbol_size,
            "k_default": self.k(0).max(if self.source_size == 0 { 0 } else { 1 }),
            "r_pct": self.overhead_pct,
            "group_bytes": self.group_bytes,
            "source_size": self.source_size,
            "group_count": self.groups,
            "total_wire": self.total_wire(),
            "overhead": self.overhead_ratio(),
        })
    }

    /// Group boundaries: `g`-th range is `[g·G, min((g+1)·G, source_size))`.
    pub fn group_ranges(&self) -> Vec<(u64, u64)> {
        (0..self.groups)
            .map(|g| {
                let start = g as u64 * self.group_bytes;
                (start, start + self.group_true_len(g))
            })
            .collect()
    }
}

/// Framing errors — loud, never silent truncation (W5: the crate's
/// `EncodingPacket::deserialize` panics on <4 B input; this layer refuses
/// such bytes before they reach it).
#[derive(Debug, thiserror::Error)]
pub enum FecError {
    /// Fewer than [`HEADER_BYTES`] bytes — not even a header.
    #[error("fec stream too short ({0} B)")]
    TooShort(usize),
    /// The packet region is not an exact multiple of `4 + symbol_size`
    /// (torn tail), or the header declares a zero symbol size.
    #[error("fec stream framing broken at byte {0}")]
    Framing(usize),
}

/// Symbolize one group into its framed wire stream:
/// `[u32 BE true_len][u16 BE symbol_size][K source packets][R repair packets]`.
///
/// `group_bytes` must be exactly [`FecGeometry::group_true_len`] long (the
/// caller slices the source file). The zero-symbol group (empty file)
/// produces the bare 6-byte header — no encoder is invoked.
pub fn symbolize_group(geo: &FecGeometry, g: u32, group_bytes: &[u8]) -> Vec<u8> {
    assert_eq!(
        group_bytes.len() as u64,
        geo.group_true_len(g),
        "caller must pass exactly group_true_len(g) bytes"
    );
    let k = geo.k(g);
    let mut out = Vec::with_capacity(geo.wire_len(g) as usize);
    out.extend_from_slice(&(group_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&geo.symbol_size.to_be_bytes());
    if k == 0 {
        return out;
    }
    let mut padded = group_bytes.to_vec();
    padded.resize(k as usize * geo.symbol_size as usize, 0);
    let cfg = ObjectTransmissionInformation::new(0, geo.symbol_size, 0, 1, 1);
    let enc = SourceBlockEncoder::new2(0, &cfg, &padded);
    for p in enc.source_packets() {
        out.extend_from_slice(&p.serialize());
    }
    let r = geo.r(g);
    if r > 0 {
        for p in enc.repair_packets(0, r) {
            out.extend_from_slice(&p.serialize());
        }
    }
    out
}

/// Parse a framed group stream into `(true_len, symbol_size, packets)`.
///
/// Length-guarded (see [`FecError`]): a torn tail is a framing ERROR — the
/// caller (relay or drill) retries the session, never guesses.
pub fn parse_stream(bytes: &[u8]) -> Result<(u64, u16, Vec<EncodingPacket>), FecError> {
    if bytes.len() < HEADER_BYTES {
        return Err(FecError::TooShort(bytes.len()));
    }
    let true_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64;
    let symbol_size = u16::from_be_bytes([bytes[4], bytes[5]]);
    if symbol_size == 0 {
        return Err(FecError::Framing(4));
    }
    let pkt = 4 + symbol_size as usize;
    let rest = &bytes[HEADER_BYTES..];
    if !rest.len().is_multiple_of(pkt) {
        return Err(FecError::Framing(HEADER_BYTES + rest.len() / pkt * pkt));
    }
    let packets = rest
        .chunks_exact(pkt)
        .map(EncodingPacket::deserialize)
        .collect();
    Ok((true_len, symbol_size, packets))
}

/// Rebuild one group's TRUE bytes from any `K` of its `K+R` packets
/// (erasure repair, zero round-trips). Returns `None` honestly when fewer
/// than `K` independent packets arrive — parity cannot invent data (W4).
/// The empty group rebuilds to an empty vec without any packets.
pub fn rebuild_group(
    true_len: u64,
    symbol_size: u16,
    packets: &[EncodingPacket],
) -> Option<Vec<u8>> {
    if true_len == 0 {
        return Some(Vec::new());
    }
    let padded_len = true_len.div_ceil(symbol_size as u64) * symbol_size as u64;
    let cfg = ObjectTransmissionInformation::new(0, symbol_size, 0, 1, 1);
    let mut dec = SourceBlockDecoder::new2(0, &cfg, padded_len);
    let out = dec.decode(packets.iter().cloned())?;
    let mut out = out;
    out.truncate(true_len as usize);
    Some(out)
}
