//! Slow-chunk eviction (spec §4.2, pattern #2 — aria2's connection
//! re-establishment, rebuilt for uploads).
//!
//! Model: every worker holding a lease samples its chunk's wire progress once
//! per [`EvictionPolicy::window`]. A worker whose throughput sits below
//! `floor_ratio × median(pool)` for `strikes_to_evict` consecutive windows is
//! evicted: its in-flight request is cancelled, the chunk returns to the pool
//! keeping its confirmed bytes, and another worker leases it and resumes from
//! the server-confirmed offset — never from zero.
//!
//! The decision logic is deliberately pure (instants and samples in, bool
//! out) so it is unit-testable without real clocks; the supervisor only wires
//! it to tokio timers.

use std::time::{Duration, Instant};

/// Eviction tuning. Defaults follow spec §4.2: 10 s windows, a floor at
/// 0.4×median, eviction on the 2nd consecutive below-floor window.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EvictionPolicy {
    /// How often each worker's wire progress is sampled.
    pub window: Duration,
    /// A worker is below the floor when its measured B/s < floor_ratio ×
    /// median(pool) (workers without a finished window do not vote).
    pub floor_ratio: f64,
    /// Consecutive below-floor windows that revoke a lease.
    pub strikes_to_evict: u32,
}

impl Default for EvictionPolicy {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(10),
            floor_ratio: 0.4,
            strikes_to_evict: 2,
        }
    }
}

/// Per-worker sliding-window throughput watch. One per lease.
#[derive(Debug)]
pub(crate) struct ThroughputWatch {
    window: Duration,
    window_start: Instant,
    last_total: u64,
    strikes: u32,
    /// The primer window has been consumed (see [`ThroughputWatch::sample`]).
    primed: bool,
}

impl ThroughputWatch {
    pub fn new(policy: &EvictionPolicy, now: Instant) -> Self {
        Self {
            window: policy.window,
            window_start: now,
            last_total: 0,
            strikes: 0,
            primed: false,
        }
    }

    /// Feed the call's cumulative wire bytes. Returns the finished window's
    /// throughput (bytes/sec) when a full window has elapsed, else `None`.
    ///
    /// The FIRST elapsed window of a lease is a PRIMER and is discarded
    /// (returns `None`, resets the accounting): it measures the kernel
    /// socket-buffer drain, not the wire. At localhost/CI scale the client's
    /// TCP buffers absorb hundreds of KB the instant a PUT opens, so the
    /// primer window reports 50–150 KB/s on a link that trickles at 8 KB/s —
    /// the long-soak receipts (issue #95 R3, run 42541835145) caught fleet
    /// medians of 461 169 / 266 615 / 72 091 B/s on an 8 192 B/s bearer, a
    /// floor above every honest streamer, and an eviction cascade that
    /// killed healthy parts until the attempt budget died. On fast links
    /// parts complete inside the burst and the artifact is invisible — which
    /// is exactly why it took an hours-shaped honest rehearsal to find.
    /// The primer's byte count still seeds `last_total`, so window 2 measures
    /// the true steady-state delta.
    pub(crate) fn sample(&mut self, now: Instant, total_wire_bytes: u64) -> Option<f64> {
        let elapsed = now.duration_since(self.window_start);
        if elapsed < self.window {
            return None;
        }
        let delta = total_wire_bytes.saturating_sub(self.last_total);
        let bps = delta as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
        self.window_start = now;
        self.last_total = total_wire_bytes;
        if !self.primed {
            self.primed = true;
            return None;
        }
        Some(bps)
    }

    pub(crate) fn strikes(&self) -> u32 {
        self.strikes
    }

    /// Account one evaluated window against the strike counter.
    pub(crate) fn record_window(&mut self, below_floor: bool) {
        if below_floor {
            self.strikes += 1;
        } else {
            self.strikes = 0;
        }
    }

    /// Enough consecutive below-floor windows to revoke this lease?
    pub(crate) fn exhausted(&self, policy: &EvictionPolicy) -> bool {
        self.strikes >= policy.strikes_to_evict
    }
}

/// Median of a sample set (workers that have not finished a window yet do not
/// vote — a fresh lease must not drag the floor down and kill healthy peers).
pub(crate) fn median(samples: &mut [f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = samples.len() / 2;
    Some(if samples.len().is_multiple_of(2) {
        (samples[mid - 1] + samples[mid]) / 2.0
    } else {
        samples[mid]
    })
}

/// Is `my_bps` below the pool floor this window?
///
/// * fewer than two sampled workers → no verdict (`false`): with the #25
///   fleet votes (completed leases keep voting their measured rate) fewer
///   than two samples means a TRUE lone fleet — e.g. TUS
///   `max_parallel = 1` — where a lone worker can never be below its own
///   median and eviction with nobody to re-lease the chunk is pointless
///   anyway;
/// * a fully idle pool (median 0) → no verdict: zero-everything is no signal,
///   evicting everyone would just thrash.
pub(crate) fn below_floor(my_bps: f64, pool_samples: &mut [f64], policy: &EvictionPolicy) -> bool {
    if pool_samples.len() < 2 {
        return false;
    }
    let Some(med) = median(pool_samples) else {
        return false;
    };
    if med <= 0.0 {
        return false;
    }
    my_bps < policy.floor_ratio * med
}

/// Pattern #9 tuning (issue #27): drive the pool's dormant `resplit_pending`
/// from measured throughput. The honest goal is feedback latency, NOT
/// bandwidth: finer parts confirm sooner, feed eviction votes sooner, and
/// waste less wire when a connection dies mid-part — they never raise the
/// link's total capacity. The floor is [`crate::chunk::CHUNK_MIN`] (the S3
/// 5 MiB non-final-part protocol minimum), so on S3 lanes the win is bounded
/// and the policy must respect it.
///
/// Deliberately SHRINK-ONLY. The grow direction (merge parts back on fast
/// links) was drafted and rejected: S3 parts confirm atomically, so bigger
/// parts mean WORSE feedback latency, and a grow retires in-flight PUTs —
/// abandoning their unconfirmed prefixes (a full re-PUT of the span on S3)
/// to save a handful of per-part requests. No measured scenario needed it;
/// wiring it would trade real wire waste for nothing on the only lanes
/// IUM ships today. If a lane with meaningful per-part overhead ever
/// appears, grow returns as its own issue with its own evidence.
///
/// Defaults shape a part to ~30 s of wire time at the observed fleet median
/// and only act when reality leaves a wide deadband — a resplit retires
/// in-flight leases, so it must never flap on median noise.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResplitPolicy {
    /// Target seconds of wire time per part at the observed median rate.
    pub target_secs: f64,
    /// Reshape (shrink) when the current largest outstanding part would take
    /// longer than this at the median rate.
    pub slow_confirm: Duration,
    /// Ignore proposals that move the target less than this fraction of the
    /// current target — the anti-flap deadband.
    pub min_change_ratio: f64,
}

impl Default for ResplitPolicy {
    fn default() -> Self {
        Self {
            target_secs: 30.0,
            slow_confirm: Duration::from_secs(90),
            min_change_ratio: 0.25,
        }
    }
}

impl ResplitPolicy {
    /// The pure decision (no clocks, no I/O — unit-testable). Inputs: the
    /// fleet median bps from the last closed window (`None` = no window has
    /// closed — never act on no data), and the pool's current outstanding
    /// target (largest Pending/Leased part; 0 = nothing outstanding).
    /// Returns the new target to resplit around, or `None` for no change.
    ///
    /// A shrink may only fire when it would ACTUALLY split something: the
    /// split keeps pieces ≥ `chunk_min`, so a part of ≤ 2×target never
    /// splits — without that gate the policy would resplit (retiring
    /// in-flight leases for zero shape change) every window in the
    /// floor-bound regime, an infinite cancel-and-restart loop.
    pub fn plan(
        &self,
        median_bps: Option<f64>,
        current_target: u64,
        chunk_min: u64,
    ) -> Option<u64> {
        let median = median_bps?;
        if !median.is_finite() || median <= 0.0 || current_target == 0 {
            return None;
        }
        let secs = current_target as f64 / median;
        if secs <= self.slow_confirm.as_secs_f64() {
            return None;
        }
        let implied = self.target_for(median, chunk_min);
        if implied >= current_target {
            return None;
        }
        // Would the split actually change the shape? A part of ≤ 2× target
        // stays whole (`resplit_pending` keeps it), and pieces below
        // `chunk_min` are dropped — at the floor regime nothing may be
        // splittable, and re-planning anyway would retire in-flight leases
        // for nothing.
        let splittable = current_target > 2 * implied;
        let moves_enough =
            (current_target - implied) as f64 / current_target as f64 >= self.min_change_ratio;
        if splittable && moves_enough {
            Some(implied)
        } else {
            None
        }
    }

    /// Throughput → part target (~`target_secs` of wire time), clamped to
    /// the protocol floor and the pool's ceiling — `suggested_chunk_size`
    /// semantics.
    fn target_for(&self, median_bps: f64, chunk_min: u64) -> u64 {
        let raw = (median_bps * self.target_secs) as u64;
        raw.clamp(chunk_min, crate::chunk::CHUNK_MAX)
    }

    /// The frozen-link rule (issue #28, cloud RED 41571828134): when the
    /// fleet median is UNMEASURABLE — every sample zero, the adapter's first
    /// write wedged inside socket buffers smaller than one report — `plan()`
    /// correctly refuses (median 0 is no data). But a job that has starved
    /// past `slow_confirm` with the same oversized part still outstanding and
    /// not one measured byte IS data: the part is definitionally too coarse
    /// for this link. Propose the protocol floor, subject to the same
    /// would-actually-split gates as `plan` (hysteresis holds in the floor
    /// regime — a piece at/below 2× the floor never splits, so the rule
    /// cannot loop).
    pub fn plan_frozen(&self, current_target: u64, chunk_min: u64) -> Option<u64> {
        if current_target == 0 {
            return None;
        }
        let implied = chunk_min; // no measurable rate — assume the worst legal shape
        let splittable = current_target > 2 * implied;
        let moves_enough =
            (current_target - implied) as f64 / current_target as f64 >= self.min_change_ratio;
        if splittable && moves_enough {
            Some(implied)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(window_ms: u64) -> EvictionPolicy {
        EvictionPolicy {
            window: Duration::from_millis(window_ms),
            floor_ratio: 0.4,
            strikes_to_evict: 2,
        }
    }

    #[test]
    fn plan_frozen_floors_an_oversized_part_and_holds_hysteresis() {
        let p = ResplitPolicy::default();
        let floor = 5 * 1024 * 1024;
        // a 22 MiB part under a frozen link → the 5 MiB protocol floor
        assert_eq!(p.plan_frozen(22 * 1024 * 1024, floor), Some(floor));
        // at the floor regime nothing may split (hysteresis — no loops)
        assert_eq!(p.plan_frozen(floor, floor), None);
        assert_eq!(
            p.plan_frozen(9 * 1024 * 1024, floor),
            None,
            "< 2× floor cannot split"
        );
        assert_eq!(
            p.plan_frozen(2 * floor, floor),
            None,
            "exactly 2× floor is the gate boundary"
        );
        assert_eq!(
            p.plan_frozen(2 * floor + 1, floor),
            Some(floor),
            "past the boundary it splits (pieces 5 MiB + remainder ≥ 5 MiB — same algebra as plan)"
        );
        assert_eq!(p.plan_frozen(0, floor), None);
    }

    #[test]
    fn watch_reports_nothing_before_the_window_elapses() {
        let p = policy(100);
        let t0 = Instant::now();
        let mut w = ThroughputWatch::new(&p, t0);
        assert!(w.sample(t0 + Duration::from_millis(50), 1_000).is_none());
        // The FIRST elapsed window is the primer (socket-buffer drain, not
        // wire — issue #95 R3): discarded, but it seeds the accounting.
        assert!(
            w.sample(t0 + Duration::from_millis(100), 2_000).is_none(),
            "the primer window must not vote"
        );
        // Window 2 counts only the delta past the primer's boundary.
        assert_eq!(
            w.sample(t0 + Duration::from_millis(250), 3_000),
            Some(1_000.0 / 0.15),
        );
        // A burst in the primer must not leak into the next window: 3 000
        // bytes at the primer boundary + a 100 KB burst mid-window 2 still
        // measures exactly the window-2 delta.
        assert_eq!(
            w.sample(t0 + Duration::from_millis(350), 103_000),
            Some(100_000.0 / 0.1),
        );
    }

    #[test]
    fn median_handles_odd_even_and_empty() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [5.0]), Some(5.0));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn lone_worker_is_never_evicted() {
        let p = policy(100);
        // even at literal zero throughput: no peers → no verdict
        assert!(!below_floor(0.0, &mut [0.0], &p));
        assert!(!below_floor(0.0, &mut [], &p));
    }

    #[test]
    fn fully_idle_pool_is_never_evicted() {
        let p = policy(100);
        assert!(!below_floor(0.0, &mut [0.0, 0.0], &p));
    }

    #[test]
    fn stalled_worker_below_floor_with_a_healthy_peer() {
        let p = policy(100);
        // healthy peer at 1000 bps, stalled worker at 0 → 0 < 0.4×1000
        assert!(below_floor(0.0, &mut [1000.0, 0.0], &p));
        // slow-but-moving under the floor also evicts
        assert!(below_floor(100.0, &mut [1000.0, 100.0], &p));
        // at the floor boundary (not below) → stays
        assert!(!below_floor(400.0, &mut [1000.0, 400.0], &p));
        // healthy worker is above the floor → stays
        assert!(!below_floor(1000.0, &mut [1000.0, 0.0], &p));
    }

    // -- ResplitPolicy (issue #27) ------------------------------------------

    fn resplit_policy() -> ResplitPolicy {
        ResplitPolicy {
            target_secs: 30.0,
            slow_confirm: Duration::from_secs(90),
            min_change_ratio: 0.25,
        }
    }

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn resplit_needs_data_and_outstanding_work() {
        let p = resplit_policy();
        // No window has closed → never act on no data.
        assert_eq!(p.plan(None, 22 * MIB, crate::chunk::CHUNK_MIN), None);
        // Zero/negative median (idle pool) → no signal.
        assert_eq!(p.plan(Some(0.0), 22 * MIB, crate::chunk::CHUNK_MIN), None);
        assert_eq!(
            p.plan(Some(f64::NAN), 22 * MIB, crate::chunk::CHUNK_MIN),
            None
        );
        // Nothing outstanding → nothing to reshape.
        assert_eq!(
            p.plan(Some(2.0 * MIB as f64), 0, crate::chunk::CHUNK_MIN),
            None
        );
    }

    #[test]
    fn resplit_shrinks_a_part_that_confirms_too_slowly() {
        let p = resplit_policy();
        // 22 MiB part at 2.4 MiB/s ≈ 9.4 s ≫ 90 s? No — pick a REALLY slow
        // link: 22 MiB at 128 KiB/s = 176 s ≫ 90 s → shrink. Implied target
        // = 128 KiB/s × 30 s = 3.75 MiB → floored to CHUNK_MIN (5 MiB).
        let slow = 128.0 * 1024.0;
        assert_eq!(
            p.plan(Some(slow), 22 * MIB, crate::chunk::CHUNK_MIN),
            Some(crate::chunk::CHUNK_MIN),
            "the implied target collapses to the S3 protocol floor"
        );
    }

    #[test]
    fn resplit_never_fires_when_the_floor_leaves_nothing_to_split() {
        let p = ResplitPolicy {
            target_secs: 0.5,
            slow_confirm: Duration::from_secs(4),
            min_change_ratio: 0.1,
        };
        let floor = crate::chunk::CHUNK_MIN;
        // A 6 MiB part at 1 MiB/s = 6 s > 4 s and implied (0.5 MiB) floored
        // to 5 MiB < 6 MiB — but 6 MiB ≤ 2×5 MiB: the split would keep the
        // part whole. Resplitting anyway would retire in-flight leases for
        // ZERO shape change, every window, forever.
        assert_eq!(p.plan(Some(1.0 * MIB as f64), 6 * MIB, floor), None);
        // 8 MiB at the same rate: still ≤ 2× floor → no resplit.
        assert_eq!(p.plan(Some(1.0 * MIB as f64), 8 * MIB, floor), None);
        // 12 MiB splits (2 × 6 MiB) → fires.
        assert_eq!(p.plan(Some(1.0 * MIB as f64), 12 * MIB, floor), Some(floor));
    }

    #[test]
    fn resplit_respects_the_deadband() {
        let p = resplit_policy();
        // 256 MiB at 1 MiB/s = 256 s > 90 s; implied 30 MiB. Change ratio
        // (256-30)/256 = 0.88 ≥ 0.25 → fires.
        assert_eq!(
            p.plan(Some(1.0 * MIB as f64), 256 * MIB, crate::chunk::CHUNK_MIN),
            Some(30 * MIB)
        );
        // 36 MiB at 1 MiB/s = 36 s > 90 s? No. At 256 KiB/s = 144 s > 90 s;
        // implied 7.5 MiB; ratio (36-7.5)/36 = 0.79 → fires.
        assert_eq!(
            p.plan(Some(256.0 * 1024.0), 36 * MIB, crate::chunk::CHUNK_MIN),
            Some((7.5 * MIB as f64) as u64),
        );
        // Sub-deadband move: 13 MiB at 512 KiB/s = 26 s < 90 s → no shrink
        // at all (band, not just deadband).
        assert_eq!(
            p.plan(Some(512.0 * 1024.0), 13 * MIB, crate::chunk::CHUNK_MIN),
            None
        );
    }

    #[test]
    fn resplit_is_shrink_only_and_never_touches_fast_fleets() {
        let p = resplit_policy();
        // The grow direction was deliberately rejected (see the policy docs):
        // merging parts on a fast link retires in-flight PUTs — abandoning
        // their unconfirmed prefixes on S3 — to save a handful of requests.
        // No rate, however fast, may trigger a resplit: 5 MiB parts at
        // 32 MiB/s confirm in 0.16 s — far inside the band → None.
        assert_eq!(
            p.plan(Some(32.0 * MIB as f64), 5 * MIB, crate::chunk::CHUNK_MIN),
            None
        );
        // Even an absurd rate with tiny parts stays untouched.
        assert_eq!(
            p.plan(Some(1024.0 * MIB as f64), MIB, crate::chunk::CHUNK_MIN),
            None
        );
    }

    #[test]
    fn resplit_stays_stable_after_its_own_shrink() {
        // Post-shrink stability: the policy must not flap. After a shrink to
        // the floor, the new outstanding target (~2× implied at most) must
        // not re-trigger: 11 MiB outstanding (2×5.5 after a split of 22),
        // median 128 KiB/s → 88 s < 90 s → no fire.
        let p = resplit_policy();
        assert_eq!(
            p.plan(Some(128.0 * 1024.0), 11 * MIB, crate::chunk::CHUNK_MIN),
            None
        );
    }

    #[test]
    fn strikes_accumulate_only_on_consecutive_bad_windows() {
        let p = policy(100);
        let mut w = ThroughputWatch::new(&p, Instant::now());
        w.record_window(true);
        assert!(!w.exhausted(&p), "one strike is not enough");
        w.record_window(true);
        assert!(w.exhausted(&p), "two consecutive strikes evict");
        // a recovered window resets the counter — flapping is tolerated
        let mut w2 = ThroughputWatch::new(&p, Instant::now());
        w2.record_window(true);
        w2.record_window(false);
        assert_eq!(w2.strikes(), 0);
        w2.record_window(true);
        assert!(!w2.exhausted(&p));
    }
}
