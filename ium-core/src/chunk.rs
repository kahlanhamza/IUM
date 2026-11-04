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

use crate::journal::ChunkRow;

/// One chunk's lifecycle within a job. Serialized to the journal's chunk
/// table; `state`/`bytes_confirmed`/`adapter_token` are journal truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkState {
    /// Not yet leased (or returned to the pool after an eviction/error).
    Pending,
    /// A worker holds a lease and is transferring its span.
    Leased,
    /// Server confirmed the whole span (the adapter's token is recorded).
    Uploaded,
    /// The verify gate accepted it (ADR-004) — the final per-chunk state.
    Verified,
}

/// One planned byte range plus its transfer progress. The unit the pool
/// leases, evicts and verifies; the union of all chunks covers the source
/// exactly (no gaps, no overlaps — invariant fuzz-pinned).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Chunk {
    /// Stable index within the job (0-based, journal key); renumbered only
    /// by an explicit resplit.
    pub idx: u32,
    /// Inclusive begin offset.
    pub begin: u64,
    /// Exclusive end offset.
    pub end: u64,
    /// Lifecycle state (journal truth).
    pub state: ChunkState,
    /// Server-confirmed bytes within this chunk.
    pub bytes_confirmed: u64,
    /// Opaque protocol token (S3 part number, tus offset snapshot, ...).
    pub adapter_token: Option<String>,
}

impl Chunk {
    /// Planned span length in bytes (0 for the 0-byte job's single chunk).
    pub fn len(&self) -> u64 {
        self.end - self.begin
    }
    /// True for the 0-byte job's single zero-length chunk.
    pub fn is_empty(&self) -> bool {
        self.begin == self.end
    }
}

/// What a worker receives when it leases a chunk.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Lease {
    /// The leased chunk's index.
    pub idx: u32,
    /// Inclusive begin offset of the chunk's span.
    pub begin: u64,
    /// Exclusive end offset of the chunk's span.
    pub end: u64,
    /// The worker MUST start transferring at this offset (server-confirmed).
    pub resume_from: u64,
    /// Pool-global attempt counter (diagnostics; the retry budget counts
    /// separately in the manager).
    pub attempt: u32,
}

/// The in-memory chunk algebra over one job: planning, leasing, eviction,
/// resplit, tail race. Every mutation keeps the cover invariant (the union
/// of chunks is exactly `[0, total_len)`); the manager journals each
/// mutation's effect through `ChunkRow`s.
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
            // Issue #5 invariant: every committed job has >= 1 chunk. A
            // 0-byte file plans one zero-length chunk that must still be
            // leased, uploaded, confirmed and verified — nothing reaches
            // Done without a server round-trip.
            return vec![(0, 0)];
        }
        let n = n.max(1);
        let n = Self::clamp_by_chunk_min(total_len, n, chunk_min);
        Self::even_split(total_len, n)
    }

    /// ADR-011 (issue #8) — adapter declares, planner obeys. Same even split
    /// as [`ChunkPool::plan`], with the adapter-declared per-call ceiling as
    /// a HARD constraint: `n` is raised to at least `ceil(total_len / cap)`
    /// so no planned chunk can exceed the cap. The existing `chunk_min`
    /// clamp still applies to the requested count, but the cap wins when the
    /// two conflict — `chunk_min` is an efficiency preference, the cap is a
    /// protocol limit; a violated limit fails uploads, a violated floor only
    /// costs requests. `None` (or a nonsensical 0) delegates to
    /// [`ChunkPool::plan`].
    pub fn plan_capped(
        total_len: u64,
        n: u32,
        chunk_min: u64,
        cap: Option<u64>,
    ) -> Vec<(u64, u64)> {
        let Some(cap) = cap.filter(|c| *c > 0) else {
            return Self::plan(total_len, n, chunk_min);
        };
        if total_len == 0 {
            // Issue #5 invariant (see `plan`).
            return vec![(0, 0)];
        }
        let n = n.max(1);
        let n_soft = Self::clamp_by_chunk_min(total_len, n, chunk_min);
        let n_hard = total_len.div_ceil(cap).min(u32::MAX as u64) as u32;
        Self::even_split(total_len, n_soft.max(n_hard))
    }

    /// The existing policy clamp: never more chunks than keep every chunk
    /// ≥ `chunk_min` (where the total length allows any at all).
    fn clamp_by_chunk_min(total_len: u64, n: u32, chunk_min: u64) -> u32 {
        if chunk_min == 0 {
            return n;
        }
        let max_by_min = if total_len >= chunk_min {
            (total_len / chunk_min) as u32
        } else {
            1
        };
        n.min(max_by_min)
    }

    /// One even split of `[0, total_len)` into exactly `n` ranges — the
    /// first `total_len % n` ranges one byte longer than the rest.
    fn even_split(total_len: u64, n: u32) -> Vec<(u64, u64)> {
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

    /// Build a pool for `[0, total_len)` split into up to `max_conns`
    /// chunks (each ≥ `CHUNK_MIN` where the total allows). No adapter cap —
    /// see [`ChunkPool::new_capped`] for the ADR-011 form.
    pub fn new(total_len: u64, max_conns: u32) -> Self {
        Self::new_capped(total_len, max_conns, None)
    }

    /// ADR-011 (issue #8): pool planned under the adapter-declared per-call
    /// cap. `cap = None` is exactly [`ChunkPool::new`].
    pub fn new_capped(total_len: u64, max_conns: u32, cap: Option<u64>) -> Self {
        Self::new_capped_floor(total_len, max_conns, CHUNK_MIN, cap)
    }

    /// ADR-011 with a LANE floor (issue #44): the smallest legal piece is
    /// the adapter's declaration, not the global `CHUNK_MIN` — on a
    /// 64 KiB-floor tus lane the planner may legally dip below the 5 MiB
    /// S3 protocol minimum (which exists only for S3 multipart). The
    /// ADR-011 composition is unchanged: `n = max(min(n, floor(total/
    /// chunk_min)), ceil(total/cap))` — the cap still wins when the two
    /// conflict, and the #27 floor-regime algebra and #5 invariants run
    /// against the lane floor.
    pub fn new_capped_floor(
        total_len: u64,
        max_conns: u32,
        chunk_min: u64,
        cap: Option<u64>,
    ) -> Self {
        let ranges = Self::plan_capped(total_len, max_conns, chunk_min, cap);
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
            // Issue #27 (floor regime): `ceil(len/target)` pieces can each
            // fall BELOW `chunk_min` (12 MiB at target 5 MiB → 3×4 MiB) —
            // every such piece was dropped and the chunk silently stayed
            // whole, exactly when a slow link needs the split most. Cap the
            // piece count by the floor: pieces = min(ceil(len/target),
            // floor(len/chunk_min)) — then every piece is ≥ chunk_min by
            // construction.
            let pieces_by_target = c.len().div_ceil(target);
            let pieces_by_floor = (c.len() / chunk_min).max(1);
            let pieces = pieces_by_target.min(pieces_by_floor).max(1);
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

    /// The pool's current resplit target (issue #27): the largest
    /// outstanding (Pending or Leased) part. Uploaded/verified chunks are
    /// finished work, not shape — a fleet waiting on one 22 MiB straggler
    /// reports 22 MiB, not the average.
    pub fn outstanding_target(&self) -> u64 {
        self.chunks
            .iter()
            .filter(|c| matches!(c.state, ChunkState::Pending | ChunkState::Leased))
            .map(|c| c.len())
            .max()
            .unwrap_or(0)
    }

    /// The job's total source length the pool was built for.
    pub fn total_len(&self) -> u64 {
        self.total_len
    }
    /// The chunk list (index order) — snapshot/inspector projection.
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
    /// Is any live lease held on this chunk (tail race may hold several)?
    pub fn is_leased(&self, idx: u32) -> bool {
        self.leases.contains_key(&idx)
    }
    /// Bytes not yet server-confirmed across the whole job — journal-truth
    /// adjacent progress math (never client optimism).
    pub fn remaining_unconfirmed(&self) -> u64 {
        self.chunks
            .iter()
            .filter(|c| c.state != ChunkState::Verified)
            .map(|c| c.len() - c.bytes_confirmed)
            .sum()
    }
    /// Every chunk has full server confirmation — the manager may enter
    /// Finishing (verify still gates Done).
    pub fn all_uploaded(&self) -> bool {
        self.chunks
            .iter()
            .all(|c| c.state == ChunkState::Uploaded || c.state == ChunkState::Verified)
    }
    /// Record the verify gate's acceptance for one chunk (ADR-004).
    pub fn mark_verified(&mut self, idx: u32) {
        self.chunks[idx as usize].state = ChunkState::Verified;
    }
    /// Persistable rows for the journal (order = idx order).
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
    fn zero_byte_file_plans_single_zero_length_chunk() {
        // Issue #5 invariant: every committed job has >= 1 chunk. A 0-byte
        // file plans exactly one [0, 0) chunk that must still be leased,
        // uploaded and confirmed — no job reaches Done without a server
        // round-trip (the old empty-plan convention let it happen silently).
        assert_eq!(ChunkPool::plan(0, 16, CHUNK_MIN), vec![(0, 0)]);
        let mut pool = ChunkPool::new(0, 8);
        assert_eq!(pool.chunks().len(), 1);
        let c = &pool.chunks()[0];
        assert_eq!((c.begin, c.end), (0, 0));
        assert_eq!(c.state, ChunkState::Pending);
        assert_eq!(c.bytes_confirmed, 0);
        let idx = c.idx;
        assert!(!pool.all_uploaded(), "0-byte chunk must still be uploaded");
        pool.complete(idx);
        assert!(pool.all_uploaded());
        assert_eq!(pool.remaining_unconfirmed(), 0);
    }

    #[test]
    fn plan_capped_raises_n_when_the_cap_binds() {
        // Issue #8 AC: 10 MiB file, max 4 MiB → >= 3 chunks, none over cap.
        const MIB: u64 = 1024 * 1024;
        let ranges = ChunkPool::plan_capped(10 * MIB, 16, CHUNK_MIN, Some(4 * MIB));
        assert!(ranges.len() >= 3, "got {} chunks", ranges.len());
        assert!(ranges.iter().all(|(b, e)| e - b <= 4 * MIB));
        assert!(ranges.iter().all(|(b, e)| e - b > 0));
        let sum: u64 = ranges.iter().map(|(b, e)| e - b).sum();
        assert_eq!(sum, 10 * MIB);
        let mut offset = 0;
        for (b, e) in &ranges {
            assert_eq!(*b, offset, "chunks must tile [0, total) with no gaps");
            offset = *e;
        }
        assert_eq!(offset, 10 * MIB);
    }

    #[test]
    fn plan_capped_not_binding_is_byte_identical_to_plan() {
        const MIB: u64 = 1024 * 1024;
        // A cap above CHUNK_MAX can never bind: output equals `plan`.
        for &(total, n) in &[
            (20 * MIB, 4u32),
            (3 * CHUNK_MIN, 2u32),
            (1024, 16u32),
            (0, 8u32),
            (CHUNK_MAX + 1, 2u32),
        ] {
            assert_eq!(
                ChunkPool::plan_capped(total, n, CHUNK_MIN, Some(512 * MIB)),
                ChunkPool::plan(total, n, CHUNK_MIN),
                "total={total} n={n}"
            );
            // `None` delegates to `plan` too.
            assert_eq!(
                ChunkPool::plan_capped(total, n, CHUNK_MIN, None),
                ChunkPool::plan(total, n, CHUNK_MIN),
                "total={total} n={n}"
            );
        }
    }

    #[test]
    fn plan_capped_cap_wins_over_chunk_min_conflict() {
        const MIB: u64 = 1024 * 1024;
        // cap 4 MiB < chunk_min 5 MiB: the hard protocol limit wins; chunks
        // dip below the policy floor rather than exceed the cap.
        let ranges = ChunkPool::plan_capped(50 * MIB, 2, CHUNK_MIN, Some(4 * MIB));
        assert!(
            ranges.len() >= 13,
            "n >= ceil(50/4) = 13, got {}",
            ranges.len()
        );
        assert!(ranges.iter().all(|(b, e)| e - b <= 4 * MIB));
        let sum: u64 = ranges.iter().map(|(b, e)| e - b).sum();
        assert_eq!(sum, 50 * MIB);
    }

    #[test]
    fn plan_capped_cap_equal_to_chunk_min_boundary() {
        const MIB: u64 = 1024 * 1024;
        // cap == chunk_min: every chunk is exactly the boundary value.
        let ranges = ChunkPool::plan_capped(10 * MIB, 16, CHUNK_MIN, Some(CHUNK_MIN));
        assert!(ranges.iter().all(|(b, e)| e - b <= CHUNK_MIN));
        let sum: u64 = ranges.iter().map(|(b, e)| e - b).sum();
        assert_eq!(sum, 10 * MIB);
    }

    #[test]
    fn plan_capped_zero_byte_file_still_plans_one_chunk() {
        // Issue #5 invariant survives the cap: [(0, 0)] regardless of cap.
        assert_eq!(
            ChunkPool::plan_capped(0, 8, CHUNK_MIN, Some(1)),
            vec![(0, 0)]
        );
    }

    #[test]
    fn new_capped_pool_respects_the_cap() {
        const MIB: u64 = 1024 * 1024;
        let pool = ChunkPool::new_capped(10 * MIB, 2, Some(4 * MIB));
        assert!(pool.chunks().len() >= 3);
        assert!(pool.chunks().iter().all(|c| c.len() <= 4 * MIB));
        // `new` stays exactly the uncapped planner.
        let plain = ChunkPool::new(10 * MIB, 2);
        assert_eq!(plain.chunks().len(), 2);
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
    fn resplit_splits_at_the_floor_regime_instead_of_staying_whole() {
        // Issue #27 RED: a 12.4 MiB pending chunk with target CHUNK_MIN
        // (5 MiB) used to compute 3 pieces of ~4.1 MiB, drop them all as
        // sub-floor, and stay whole — exactly when a slow link needs the
        // split most. The floor-capped piece count makes it 2 × 6.2 MiB.
        const MIB: u64 = 1024 * 1024;
        let mut pool = ChunkPool::new(12 * MIB + 2 * MIB / 5, 1); // 12.4 MiB, 1 chunk
        assert_eq!(pool.chunks().len(), 1);
        let l = pool.lease("w").unwrap();
        pool.evict(l.idx, "w"); // back to Pending, zero confirmed
        pool.resplit_pending(CHUNK_MIN, CHUNK_MIN);
        assert_cover_invariant(&pool);
        let pieces: Vec<_> = pool.chunks().to_vec();
        assert_eq!(
            pieces.len(),
            2,
            "12.4 MiB at a 5 MiB target must split into 2 floor-respecting pieces: {pieces:?}"
        );
        assert!(pieces.iter().all(|c| c.len() >= CHUNK_MIN));
        assert_eq!(pieces[0].begin, 0);
        assert_eq!(pieces[1].end, pool.total_len(), "no byte lost in the split");
        assert!(
            pieces.windows(2).all(|w| w[0].end == w[1].begin),
            "contiguous cover"
        );
    }

    #[test]
    fn outstanding_target_reports_the_largest_unfinished_part() {
        // 3 chunks of 4 MiB… CHUNK_MIN forbids that plan — build rows directly.
        let mk = |idx: u32, begin: u64, end: u64, state: ChunkState| ChunkRow {
            idx,
            begin,
            end,
            state,
            bytes_confirmed: 0,
            adapter_token: None,
        };
        let rows = vec![
            mk(0, 0, CHUNK_MIN, ChunkState::Uploaded),
            mk(1, CHUNK_MIN, 2 * CHUNK_MIN, ChunkState::Leased),
            mk(2, 2 * CHUNK_MIN, 4 * CHUNK_MIN, ChunkState::Pending),
        ];
        let pool = ChunkPool::from_rows(4 * CHUNK_MIN, rows);
        // The Uploaded 5 MiB chunk is finished work — not shape. The largest
        // outstanding is the 10 MiB pending straggler.
        assert_eq!(pool.outstanding_target(), 2 * CHUNK_MIN);
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
        // Env-gated for the nightly CI run (issue #4): IUM_FUZZ_ROUNDS /
        // IUM_FUZZ_OPS expand the run without touching the test; the suite
        // defaults (25 x 400) keep PR latency unchanged.
        let rounds: u64 = std::env::var("IUM_FUZZ_ROUNDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25);
        let ops: u64 = std::env::var("IUM_FUZZ_OPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) % m.max(1)
        };

        for round in 0..rounds {
            let total = CHUNK_MIN * (1 + next(12));
            let mut pool = ChunkPool::new(total, (1 + next(6)) as u32);
            let workers = ["w1", "w2", "w3"];
            // worker -> leased idx (single lease per worker keeps the fuzz simple)
            let mut held: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();

            for _ in 0..ops {
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
