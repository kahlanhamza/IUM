//! Chunk algebra — the upload twin of the download segment pool
//! (aria2 `SegmentMan` / FDM `IndependentSegmentsDownloader` patterns).
//!
//! Invariants maintained by every operation:
//! * the union of all chunk ranges is exactly `[0, total_len)`, no gaps, no overlaps;
//! * `bytes_confirmed` counts only server-acknowledged bytes;
//! * eviction keeps confirmed bytes (a re-leased chunk resumes, never restarts).

/// Smallest chunk we will ever plan (5 MiB, spec §4.1).
pub const CHUNK_MIN: u64 = 5 * 1024 * 1024;
/// Largest chunk target (256 MiB, spec §4.1).
pub const CHUNK_MAX: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkState {
    Pending,
    Leased,
    Uploaded,
    Verified,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Chunk {
    pub idx: u32,
    /// Inclusive begin offset.
    pub begin: u64,
    /// Exclusive end offset.
    pub end: u64,
    pub state: ChunkState,
    /// Server-confirmed bytes within this chunk.
    pub bytes_confirmed: u64,
    /// Opaque protocol token (S3 part number, tus offset snapshot, ...).
    pub adapter_token: Option<String>,
}

impl Chunk {
    pub fn len(&self) -> u64 {
        self.end - self.begin
    }
    pub fn is_empty(&self) -> bool {
        self.begin == self.end
    }
}

/// What a worker receives when it leases a chunk.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Lease {
    pub idx: u32,
    pub begin: u64,
    pub end: u64,
    /// The worker MUST start transferring at this offset (server-confirmed).
    pub resume_from: u64,
    pub attempt: u32,
}

use crate::journal::ChunkRow;

#[derive(Debug)]
pub struct ChunkPool {
    total_len: u64,
    chunks: Vec<Chunk>,
    /// idx -> worker ids currently holding a lease (multiple during tail race).
    leases: std::collections::HashMap<u32, Vec<String>>,
    next_attempt: u32,
}

impl ChunkPool {
    /// Plan `n` chunks covering `[0, total_len)` with every chunk ≥ `chunk_min`
    /// where the total length allows it.
    pub fn plan(total_len: u64, n: u32, chunk_min: u64) -> Vec<(u64, u64)> {
        if total_len == 0 {
            return Vec::new();
        }
        let n = n.max(1);
        let n = if chunk_min > 0 {
            let max_by_min = if total_len >= chunk_min {
                (total_len / chunk_min) as u32
            } else {
                1
            };
            n.min(max_by_min)
        } else {
            n
        };
        let base = total_len / n as u64;
        let rem = total_len % n as u64;
        let mut ranges = Vec::with_capacity(n as usize);
        let mut offset = 0u64;
        for i in 0..n {
            let extra = if (i as u64) < rem { 1 } else { 0 };
            let len = base + extra;
            ranges.push((offset, offset + len));
            offset += len;
        }
        ranges
    }

    pub fn new(total_len: u64, max_conns: u32) -> Self {
        let ranges = Self::plan(total_len, max_conns, CHUNK_MIN);
        let chunks = ranges
            .into_iter()
            .enumerate()
            .map(|(i, (b, e))| Chunk {
                idx: i as u32,
                begin: b,
                end: e,
                state: ChunkState::Pending,
                bytes_confirmed: 0,
                adapter_token: None,
            })
            .collect();
        Self {
            total_len,
            chunks,
            leases: std::collections::HashMap::new(),
            next_attempt: 0,
        }
    }

    /// Rebuild a pool from persisted rows (journal replay after a crash).
    pub fn from_rows(total_len: u64, rows: Vec<ChunkRow>) -> Self {
        let mut chunks: Vec<Chunk> = rows
            .into_iter()
            .map(|r| Chunk {
                idx: r.idx,
                begin: r.begin,
                end: r.end,
                state: r.state,
                bytes_confirmed: r.bytes_confirmed,
                adapter_token: r.adapter_token,
            })
            .collect();
        chunks.sort_by_key(|c| c.idx);
        Self {
            total_len,
            chunks,
            leases: std::collections::HashMap::new(),
            next_attempt: 0,
        }
    }

    /// Lease the next chunk for a worker.
    /// Preference: partially-confirmed chunks first (finish resume work early),
    /// then lowest pending index.
    pub fn lease(&mut self, worker: &str) -> Option<Lease> {
        let idx = self
            .chunks
            .iter()
            .filter(|c| c.state == ChunkState::Pending && !self.is_leased(c.idx))
            .min_by_key(|c| (if c.bytes_confirmed > 0 { 0u8 } else { 1u8 }, c.idx))
            .map(|c| c.idx)?;

        let attempt = self.next_attempt;
        self.next_attempt += 1;
        let (begin, end, resume_from) = {
            let c = &mut self.chunks[idx as usize];
            c.state = ChunkState::Leased;
            (c.begin, c.end, c.begin + c.bytes_confirmed)
        };
        self.leases.entry(idx).or_default().push(worker.to_string());
        tracing::debug!(idx, worker, "chunk leased");
        Some(Lease {
            idx,
            begin,
            end,
            resume_from,
            attempt,
        })
    }

    /// Report server-confirmed progress on a leased chunk.
    /// Panics in debug builds if `bytes` exceeds the chunk length (programming error).
    pub fn report_confirmed(&mut self, idx: u32, bytes: u64) {
        let c = &mut self.chunks[idx as usize];
        debug_assert!(
            bytes <= c.len(),
            "confirmed {} > chunk len {}",
            bytes,
            c.len()
        );
        c.bytes_confirmed = bytes.min(c.len());
    }

    /// A worker finished its lease with full server confirmation.
    pub fn complete(&mut self, idx: u32) {
        let c = &mut self.chunks[idx as usize];
        c.bytes_confirmed = c.len();
        c.state = ChunkState::Uploaded;
        self.leases.remove(&idx);
    }

    /// Evict a worker's lease (slow-chunk eviction, pattern #2).
    /// The chunk returns to the pool keeping its confirmed bytes.
    pub fn evict(&mut self, idx: u32, worker: &str) {
        let Some(holders) = self.leases.get_mut(&idx) else {
            return;
        };
        holders.retain(|w| w != worker);
        if !holders.is_empty() {
            return; // tail-race: another worker still holds it
        }
        self.leases.remove(&idx);
        let c = &mut self.chunks[idx as usize];
        if c.state == ChunkState::Leased {
            c.state = ChunkState::Pending;
        }
    }

    /// Tail race (pattern #3): when the unconfirmed remainder is small enough,
    /// every idle worker may duplicate the outstanding chunks. Returns the
    /// chunk indexes worth racing, or an empty vec if the pool isn't in endgame.
    pub fn tail_race_candidates(&self, chunk_min: u64) -> Vec<u32> {
        let remaining: u64 = self
            .chunks
            .iter()
            .filter(|c| c.state != ChunkState::Verified)
            .map(|c| c.len() - c.bytes_confirmed)
            .sum();
        if remaining > 2 * chunk_min {
            return Vec::new();
        }
        self.chunks
            .iter()
            .filter(|c| c.state == ChunkState::Leased || c.state == ChunkState::Pending)
            .map(|c| c.idx)
            .collect()
    }

    /// Re-plan pending chunks around a target size (dynamic resizing, pattern #9):
    /// split oversized pending chunks, merge undersized adjacent ones.
    /// Uploaded/verified chunks are never touched.
    ///
    /// A resplit is a supervision-level decision: outstanding in-memory leases
    /// are retired and leased chunks return to `Pending` KEEPING their
    /// confirmed bytes (the next lease resumes from `resume_from`). Leaving
    /// them `Leased` with a dead worker would strand the pool — caught by the
    /// invariant fuzz.
    pub fn resplit_pending(&mut self, target: u64, chunk_min: u64) {
        let target = target.clamp(chunk_min, CHUNK_MAX.max(chunk_min));

        // 0. Retire in-flight leases: leased chunks rejoin the pool (keeping
        //    their confirmed bytes) because their lease identity dies here.
        for c in self.chunks.iter_mut() {
            if c.state == ChunkState::Leased {
                c.state = ChunkState::Pending;
            }
        }
        self.leases.clear();

        // 1. Split oversized pending chunks.
        let mut next_idx = self.chunks.len() as u32;
        let mut rebuilt: Vec<Chunk> = Vec::with_capacity(self.chunks.len());
        for c in std::mem::take(&mut self.chunks) {
            if c.state != ChunkState::Pending || c.len() <= 2 * target {
                rebuilt.push(c);
                continue;
            }
            // Confirmed bytes only exist on chunks that were touched; a Pending
            // chunk with confirmed bytes was evicted mid-flight: keep it whole
            // so its confirmation is not scattered.
            if c.bytes_confirmed > 0 {
                rebuilt.push(c);
                continue;
            }
            let pieces = c.len().div_ceil(target);
            let piece_len = c.len() / pieces;
            let mut offset = c.begin;
            for p in 0..pieces {
                let end = if p == pieces - 1 {
                    c.end
                } else {
                    offset + piece_len
                };
                if end - offset >= chunk_min || p == pieces - 1 {
                    rebuilt.push(Chunk {
                        idx: next_idx,
                        begin: offset,
                        end,
                        state: ChunkState::Pending,
                        bytes_confirmed: 0,
                        adapter_token: None,
                    });
                    next_idx += 1;
                    offset = end;
                }
            }
            debug_assert_eq!(offset, c.end);
        }
        self.chunks = rebuilt;

        // 2. Merge adjacent pending chunks below target.
        let mut merged: Vec<Chunk> = Vec::with_capacity(self.chunks.len());
        for c in std::mem::take(&mut self.chunks) {
            match merged.last_mut() {
                Some(prev)
                    if prev.state == ChunkState::Pending
                        && c.state == ChunkState::Pending
                        && prev.end == c.begin
                        && prev.bytes_confirmed == 0
                        && c.bytes_confirmed == 0
                        && prev.len() + c.len() <= target =>
                {
                    prev.end = c.end;
                }
                _ => merged.push(c),
            }
        }
        // Renumber for stable persistence.
        for (i, c) in merged.iter_mut().enumerate() {
            c.idx = i as u32;
        }
        // Renumbering invalidates every in-memory lease index: a lease held on
        // old idx N would now "protect" an unrelated chunk (found by the
        // invariant fuzz). Callers re-lease from the rebuilt pool.
        self.leases.clear();
        self.chunks = merged;
    }

    /// Suggested next chunk target for observed average throughput
    /// (~30 s of transfer per chunk, clamped to [min, max]) — pattern #9.
    pub fn suggested_chunk_size(avg_bps: f64, chunk_min: u64) -> u64 {
        let raw = (avg_bps * 30.0) as u64;
        raw.clamp(chunk_min, CHUNK_MAX)
    }

    pub fn total_len(&self) -> u64 {
        self.total_len
    }
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
    pub fn is_leased(&self, idx: u32) -> bool {
        self.leases.contains_key(&idx)
    }
    pub fn remaining_unconfirmed(&self) -> u64 {
        self.chunks
            .iter()
            .filter(|c| c.state != ChunkState::Verified)
            .map(|c| c.len() - c.bytes_confirmed)
            .sum()
    }
    pub fn all_uploaded(&self) -> bool {
        self.chunks
            .iter()
            .all(|c| c.state == ChunkState::Uploaded || c.state == ChunkState::Verified)
    }
    pub fn mark_verified(&mut self, idx: u32) {
        self.chunks[idx as usize].state = ChunkState::Verified;
    }
    pub fn rows(&self) -> Vec<ChunkRow> {
        self.chunks
            .iter()
            .map(|c| ChunkRow {
                idx: c.idx,
                begin: c.begin,
                end: c.end,
                state: c.state,
                bytes_confirmed: c.bytes_confirmed,
                adapter_token: c.adapter_token.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_cover_invariant(pool: &ChunkPool) {
        let total = pool.total_len();
        let mut offset = 0u64;
        for c in pool.chunks() {
            assert_eq!(c.begin, offset, "gap/overlap at chunk {}", c.idx);
            assert!(c.end >= c.begin);
            assert!(c.bytes_confirmed <= c.len());
            offset = c.end;
        }
        assert_eq!(offset, total, "chunks must cover [0, total)");
    }

    #[test]
    fn plan_covers_exactly_and_respects_min() {
        let total = 100 * 1024 * 1024;
        let ranges = ChunkPool::plan(total, 16, CHUNK_MIN);
        assert!(ranges.iter().all(|(b, e)| e - b >= CHUNK_MIN));
        let sum: u64 = ranges.iter().map(|(b, e)| e - b).sum();
        assert_eq!(sum, total);
        // tiny file → single chunk even if many conns
        let ranges = ChunkPool::plan(1024, 16, CHUNK_MIN);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (0, 1024));
    }

    #[test]
    fn zero_byte_file_has_no_chunks() {
        let pool = ChunkPool::new(0, 8);
        assert!(pool.chunks().is_empty());
        assert!(pool.all_uploaded());
    }

    #[test]
    fn lease_prefers_partially_confirmed_then_lowest_idx() {
        let mut pool = ChunkPool::new(10 * CHUNK_MIN, 4);
        // complete nothing; lease all 4
        for i in 0..4 {
            let l = pool.lease("w1").unwrap();
            assert_eq!(l.idx, i);
        }
        assert!(pool.lease("w1").is_none(), "no pending chunks left");

        // evict chunk 2 mid-flight with confirmed progress
        pool.report_confirmed(2, CHUNK_MIN / 2);
        pool.evict(2, "w1");
        assert_eq!(pool.chunks()[2].state, ChunkState::Pending);
        assert_eq!(pool.chunks()[2].bytes_confirmed, CHUNK_MIN / 2);

        // next lease must hand back chunk 2 with resume_from at the confirmed offset
        let l = pool.lease("w2").unwrap();
        assert_eq!(l.idx, 2);
        assert_eq!(l.resume_from, pool.chunks()[2].begin + CHUNK_MIN / 2);
        assert_cover_invariant(&pool);
    }

    #[test]
    fn eviction_keeps_confirmed_bytes_and_releases_lease() {
        let mut pool = ChunkPool::new(2 * CHUNK_MIN, 2);
        let l = pool.lease("w1").unwrap();
        pool.report_confirmed(l.idx, 1024);
        pool.evict(l.idx, "w1");
        assert!(!pool.is_leased(l.idx));
        assert_eq!(pool.chunks()[l.idx as usize].bytes_confirmed, 1024);
        assert_cover_invariant(&pool);
    }

    #[test]
    fn resplit_merges_and_splits_while_holding_invariants() {
        // 40 chunks of CHUNK_MIN each
        let total = 40 * CHUNK_MIN;
        let mut pool = ChunkPool::new(total, 40);

        // lease + upload the first two so they are immutable
        for _ in 0..2 {
            let l = pool.lease("w").unwrap();
            pool.complete(l.idx);
        }

        // resplit pending around 4*CHUNK_MIN: should merge ~38 chunks into ~10
        pool.resplit_pending(4 * CHUNK_MIN, CHUNK_MIN);
        assert_cover_invariant(&pool);
        let pending: Vec<_> = pool
            .chunks()
            .iter()
            .filter(|c| c.state == ChunkState::Pending)
            .collect();
        assert!(pending.iter().all(|c| c.len() <= 4 * CHUNK_MIN));
        assert_eq!(pool.chunks()[0].state, ChunkState::Uploaded);
        assert_eq!(pool.chunks()[1].state, ChunkState::Uploaded);

        // now split again around CHUNK_MIN: pending pieces go small
        pool.resplit_pending(CHUNK_MIN, CHUNK_MIN);
        assert_cover_invariant(&pool);
        assert!(pool
            .chunks()
            .iter()
            .filter(|c| c.state == ChunkState::Pending)
            .all(|c| c.len() <= 2 * CHUNK_MIN));
    }

    #[test]
    fn tail_race_only_kicks_in_at_endgame() {
        let total = 8 * CHUNK_MIN;
        let mut pool = ChunkPool::new(total, 8);
        assert!(
            pool.tail_race_candidates(CHUNK_MIN).is_empty(),
            "too much left"
        );

        // upload all but two chunks worth ~1 CHUNK_MIN remainder
        for i in 0..6 {
            pool.lease("w").unwrap();
            pool.complete(i);
        }
        // mark uploaded ones verified
        for i in 0..6 {
            pool.mark_verified(i);
        }
        let idxs = pool.tail_race_candidates(CHUNK_MIN);
        assert_eq!(idxs.len(), 2, "endgame: the two tail chunks get raced");
    }

    #[test]
    fn suggested_chunk_size_clamps() {
        assert_eq!(ChunkPool::suggested_chunk_size(1.0, CHUNK_MIN), CHUNK_MIN);
        assert_eq!(
            ChunkPool::suggested_chunk_size(100.0 * 1024.0 * 1024.0 * 1024.0, CHUNK_MIN),
            CHUNK_MAX
        );
        assert_eq!(
            ChunkPool::suggested_chunk_size(1024.0 * 1024.0, CHUNK_MIN),
            30 * 1024 * 1024
        );
    }

    /// Deterministic LCG fuzz: thousands of random lease/confirm/evict/
    /// complete/resplit sequences must never break the pool invariants —
    /// exact coverage of [0, total), confirmed ≤ len, and conservation of
    /// bytes through every eviction.
    #[test]
    fn fuzz_random_operations_never_break_invariants() {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) % m.max(1)
        };

        for round in 0..25u64 {
            let total = CHUNK_MIN * (1 + next(12));
            let mut pool = ChunkPool::new(total, (1 + next(6)) as u32);
            let workers = ["w1", "w2", "w3"];
            // worker -> leased idx (single lease per worker keeps the fuzz simple)
            let mut held: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();

            for _ in 0..400 {
                assert_cover_invariant(&pool);
                match next(10) {
                    0..=4 => {
                        let w = workers[next(3) as usize];
                        if held.contains_key(w) {
                            continue;
                        }
                        if let Some(l) = pool.lease(w) {
                            held.insert(w, l.idx);
                        }
                    }
                    5 => {
                        // confirm a random prefix of a held chunk
                        let ws: Vec<&str> = held.keys().copied().collect();
                        if ws.is_empty() {
                            continue;
                        }
                        let w = ws[next(ws.len() as u64) as usize];
                        let idx = held[w];
                        let len = pool.chunks()[idx as usize].len();
                        pool.report_confirmed(idx, next(len + 1));
                    }
                    6 => {
                        // evict a random holder
                        let ws: Vec<&str> = held.keys().copied().collect();
                        if ws.is_empty() {
                            continue;
                        }
                        let w = ws[next(ws.len() as u64) as usize];
                        let idx = held.remove(&w).unwrap();
                        pool.evict(idx, w);
                    }
                    7 => {
                        // complete a random holder (server acked everything)
                        let ws: Vec<&str> = held.keys().copied().collect();
                        if ws.is_empty() {
                            continue;
                        }
                        let w = ws[next(ws.len() as u64) as usize];
                        let idx = held.remove(&w).unwrap();
                        pool.complete(idx);
                    }
                    8 => {
                        pool.resplit_pending(CHUNK_MIN * (1 + next(4)), CHUNK_MIN);
                        // resplit renumbers; live leases keep ranges but idx may
                        // drift — drop our tracking, the pool is self-consistent
                        held.clear();
                    }
                    _ => {
                        // journal round-trip: persist + rebuild must be lossless
                        let rows = pool.rows();
                        let rebuilt = ChunkPool::from_rows(pool.total_len(), rows);
                        assert_cover_invariant(&rebuilt);
                        assert_eq!(
                            rebuilt.remaining_unconfirmed(),
                            pool.remaining_unconfirmed()
                        );
                        pool = rebuilt;
                    }
                }
            }

            // drain: evict everyone, then single-worker to completion
            for (w, idx) in held.drain() {
                pool.evict(idx, w);
            }
            while !pool.all_uploaded() {
                assert_cover_invariant(&pool);
                let Some(l) = pool.lease("drain") else {
                    pool.resplit_pending(CHUNK_MIN, CHUNK_MIN);
                    let before = pool.remaining_unconfirmed();
                    let Some(l) = pool.lease("drain") else {
                        panic!("round {round}: pool wedged with {} bytes left", before);
                    };
                    pool.complete(l.idx);
                    continue;
                };
                pool.complete(l.idx);
            }
            assert_eq!(pool.remaining_unconfirmed(), 0);
        }
    }
}
