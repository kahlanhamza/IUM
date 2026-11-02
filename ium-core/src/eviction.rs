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
}

impl ThroughputWatch {
    pub fn new(policy: &EvictionPolicy, now: Instant) -> Self {
        Self {
            window: policy.window,
            window_start: now,
            last_total: 0,
            strikes: 0,
        }
    }

    /// Feed the call's cumulative wire bytes. Returns the finished window's
    /// throughput (bytes/sec) when a full window has elapsed, else `None`.
    pub(crate) fn sample(&mut self, now: Instant, total_wire_bytes: u64) -> Option<f64> {
        let elapsed = now.duration_since(self.window_start);
        if elapsed < self.window {
            return None;
        }
        let delta = total_wire_bytes.saturating_sub(self.last_total);
        let bps = delta as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
        self.window_start = now;
        self.last_total = total_wire_bytes;
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
/// * fewer than two sampled workers → no verdict (`false`): a lone worker can
///   never be below its own median, and eviction with nobody to re-lease the
///   chunk is pointless anyway;
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
    fn watch_reports_nothing_before_the_window_elapses() {
        let p = policy(100);
        let t0 = Instant::now();
        let mut w = ThroughputWatch::new(&p, t0);
        assert!(w.sample(t0 + Duration::from_millis(50), 1_000).is_none());
        assert_eq!(
            w.sample(t0 + Duration::from_millis(100), 2_000),
            Some(2_000.0 / 0.1),
            "2000 bytes over the first 100 ms window"
        );
        // next window counts only the delta
        assert_eq!(
            w.sample(t0 + Duration::from_millis(250), 3_000),
            Some(1_000.0 / 0.15),
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
