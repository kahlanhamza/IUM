//! The global rate bucket (#94): the engine's one bandwidth-politeness knob.
//!
//! Vocabulary (from the #46 research track): nothing beats the sum of the
//! summoned bearers — so the cap is GLOBAL and divides that sum. A per-flow
//! cap would let the fleet multiply it; this bucket is shared by every
//! worker of every job, and its budget refills at exactly `cap` bytes/sec.
//!
//! Shape notes:
//! - **Lazy refill, no background task.** Under the lock,
//!   `available = min(cap, available + elapsed·cap)`; a waiter sleeps the
//!   shortfall and re-checks. Deterministic under tokio's paused virtual
//!   time, and an idle bucket costs nothing.
//! - **The bucket starts full** (one cap's worth of burst allowance) — a
//!   fresh process may immediately use one cap-second of bytes.
//! - **Unlimited = `None`** and the acquire path is a no-op: the uncapped
//!   engine pays nothing for the limiter's existence.
//! - **`set_rate` is the hot path**: the user drags the cap while jobs run;
//!   the next acquire simply obeys the new rate. (Where the pending sleep
//!   is concerned, a re-check under the new rate happens on wake — the
//!   shortfall math is recomputed from `available` and `last`, so a swap
//!   never strands a waiter at the old rate for more than one sleep
//!   quantum.)
//!
//! Eviction-safety lives at the CALL SITE (the manager acquires BETWEEN
//! leases, outside the eviction-tracked upload window — a capped wait can
//! never be sampled as a stalled chunk); this module only paces bytes.

use std::sync::Mutex;
use std::time::Duration;

/// The shared global bucket. `cap: None` = unlimited.
pub struct Bucket {
    inner: Mutex<Inner>,
}

struct Inner {
    cap: Option<u64>,
    /// Bytes available right now (f64: fractional refill accumulates).
    available: f64,
    /// When `available` was last computed.
    last: tokio::time::Instant,
}

impl Bucket {
    /// `cap` in bytes/sec; `None` = unlimited (every acquire returns at
    /// once, zero overhead).
    pub fn new(cap: Option<u64>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                cap,
                available: cap.map(|c| c as f64).unwrap_or(f64::INFINITY),
                last: tokio::time::Instant::now(),
            }),
        }
    }

    /// The current cap, for call sites that skip work entirely when
    /// unlimited (`None`).
    pub fn cap(&self) -> Option<u64> {
        self.inner.lock().expect("rate bucket poisoned").cap
    }

    /// The hot knob: swap the cap (or lift it). Running workers obey on
    /// their NEXT acquire — the wait between leases is where the pacing
    /// lives, so no in-flight chunk is interrupted.
    pub fn set_rate(&self, cap: Option<u64>) {
        let mut g = self.inner.lock().expect("rate bucket poisoned");
        g.cap = cap;
        g.available = cap.map(|c| c as f64).unwrap_or(f64::INFINITY);
        g.last = tokio::time::Instant::now();
    }

    /// Acquire `n` bytes, pacing to the cap. Cancellation-safe: the caller
    /// runs this inside its own select! (abort/resplit), so a capped wait
    /// aborts with the job.
    ///
    /// The grant is SLICED: a request larger than one cap-second (a 5 MiB
    /// lease under a 50 KB/s cap) would deadlock a single-shot bucket —
    /// the burst allowance can never cover it. Slicing grants
    /// available-sized pieces per refill and sleeps the shortfall, so any
    /// request size completes in exactly its fair `n/cap` time (minus the
    /// burst allowance) and the bucket never holds more than one burst.
    pub async fn acquire(&self, n: u64) {
        let mut remaining = n;
        while remaining > 0 {
            let wait = {
                let mut g = self.inner.lock().expect("rate bucket poisoned");
                let Some(cap) = g.cap else { return };
                let now = tokio::time::Instant::now();
                let elapsed = now.duration_since(g.last).as_secs_f64();
                g.available = (g.available + elapsed * cap as f64).min(cap as f64);
                g.last = now;
                let grant = (g.available as u64).min(remaining);
                if grant > 0 {
                    g.available -= grant as f64;
                    remaining -= grant;
                }
                if remaining == 0 {
                    return;
                }
                // The wait for the next slice: the bucket refills at `cap`
                // B/s and clamps at one burst — sleeping the WHOLE deficit
                // would overshoot (a 1900 B deficit at 100 B/s waits 19 s
                // but only 100 B can exist). Target the smaller of the
                // remaining need and one burst; the loop grants per refill.
                let target = (remaining as f64).min(cap as f64);
                Duration::from_secs_f64((target - g.available) / cap as f64)
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_bucket_reports_no_cap() {
        let b = Bucket::new(None);
        // The knob reads back: None in, None out.
        assert!(b.inner.lock().unwrap().cap.is_none());
    }

    #[test]
    fn set_rate_swaps_the_cap_and_refills_the_allowance() {
        let b = Bucket::new(Some(100));
        b.set_rate(Some(50));
        {
            let g = b.inner.lock().unwrap();
            assert_eq!(g.cap, Some(50));
            assert_eq!(g.available, 50.0, "the allowance resets to one burst");
        }
        b.set_rate(None);
        assert!(b.inner.lock().unwrap().cap.is_none());
    }
}
