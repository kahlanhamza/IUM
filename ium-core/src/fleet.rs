//! The fleet decision algebra (issues #48/#49, M8 rungs 1–2): PURE, offline,
//! zero I/O — every arm unit-tested in `tests/aimd_fleet.rs`. The manager
//! feeds it measured evidence; the journal keeps the decisions.
//!
//! Two questions live here:
//!
//! 1. **The seed** ([`fleet_for`]): how many concurrent workers may run at
//!    all — the composition of the pending count, the engine ceiling, the
//!    adapter's declared protocol bound (ADR-011) and the prober's measured
//!    recommendation (ADR-011's spirit: measure, declare, obey). The
//!    tightest bound wins; a declared 0 is nonsense and clamps to 1 (a
//!    violated floor costs requests, a violated limit fails uploads).
//!
//! 2. **The adaptation** ([`AdaptiveFleet`]): the owner's link swings 54x
//!    within one hour (3 KB/s moment vs 1.29 Mbps). Shrink-only machinery
//!    (eviction, resplit, the frozen rule) answers the down-swing; this
//!    module answers the up-swing — additive increase while measured
//!    goodput improves, multiplicative decrease when errors say the bearer
//!    (or the server) is drowning, a hard stop at the plateau, and two
//!    feedback gates: a window that moved too few bytes cannot judge
//!    goodput, and a window with too few completed calls cannot judge the
//!    error rate. The manager evaluates one window per tick; workers above
//!    the waterline park at their lease boundary and wake on growth, so
//!    BOTH directions are live within one drive round.
//!
//! Honest scope notes:
//! * The brief's RTT-inflation watch (bufferbloat) is not implementable
//!   yet — the adapters do not expose per-request latency. Error rate and
//!   the plateau carry the decrease signal; RTT rides a future delta.
//! * The 3 KB/s S3 regime (atomic parts confirm nothing per window)
//!   belongs to resplit (#27/#28), not AIMD — the byte gate hands those
//!   windows through untouched.
//! * The first evidence window SETS the baseline and holds: a fleet seeded
//!   from a measurement must not grow on its own first heartbeat.

use serde::{Deserialize, Serialize};

/// The seed composition (see the module docs). `declared`/`measured` of
/// `None` mean "no bound declared" — the ceiling stands.
pub fn fleet_for(
    pending: usize,
    max_workers: usize,
    declared: Option<u32>,
    measured: Option<u32>,
) -> usize {
    let ceiling = declared
        .map(|d| d.max(1) as usize)
        .unwrap_or(max_workers)
        .min(max_workers);
    let measured = measured
        .map(|m| m.max(1) as usize)
        .unwrap_or(ceiling)
        .min(ceiling);
    pending.min(ceiling).min(measured)
}

/// Why the fleet moved (journaled verbatim in `fleet_adjusted`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetReason {
    /// Measured goodput cleared best×1.08: the bearer has room, add streams.
    Growth,
    /// Error rate crossed the decrease line: the bearer or server is
    /// drowning — back off multiplicatively.
    Errors,
    /// Goodput fell below best×0.9 with no errors: we overshot — give one
    /// stream back.
    Overshoot,
}

/// One fleet adjustment — the manager journals this verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetChange {
    /// Fleet size before the adjustment.
    pub from: u32,
    /// Fleet size after the adjustment (already clamped to `[1, ceiling]`).
    pub to: u32,
    /// Why it moved.
    pub reason: FleetReason,
    /// The window's measured goodput (bytes/second) the decision saw.
    pub goodput_bps: f64,
}

/// The live, adaptive fleet size for one job.
///
/// Seeded from the journal (last `fleet_adjusted`, else the measured
/// `recommended_fleet`, else the ceiling) and stepped once per evidence
/// window. `None` from [`AdaptiveFleet::step`] means "hold" — no journal
/// event, no change.
#[derive(Debug, Clone)]
pub struct AdaptiveFleet {
    current: u32,
    /// The best goodput measured so far (bytes/second); 0.0 = no baseline yet.
    best_goodput: f64,
}

/// Goodput gate: windows moving fewer bytes than this carry no goodput
/// verdict (the atomic-S3 slow regime — resplit's domain).
const FEEDBACK_MIN_BYTES: u64 = 64 * 1024;
/// Goodput gate: windows shorter than this carry no goodput verdict
/// (sub-second samples would let scheduling noise drive the policy).
const FEEDBACK_MIN_SECS: f64 = 0.5;
/// Error gate: windows with fewer completed calls than this carry no error
/// verdict — one unlucky call is not a drowning bearer.
const FEEDBACK_MIN_CALLS: u32 = 4;
/// Error rate above which the fleet decreases multiplicatively.
const ERR_RATE_DECREASE: f64 = 0.3;
/// "Meaningful improvement" over the baseline (the brief's 8%).
const GROWTH_RATIO: f64 = 1.08;
/// "Meaningful regression" under the baseline (the brief's 10%).
const OVERSHOOT_RATIO: f64 = 0.9;
/// Additive-increase step (streams per growth decision).
const GROWTH_STEP: u32 = 2;
/// Multiplicative-decrease factor (the brief's 0.7).
const DECREASE_FACTOR: f64 = 0.7;

impl AdaptiveFleet {
    /// Seed at `current` — the caller clamps to the ceiling first (the
    /// journal-truth seed: last `fleet_adjusted` | measured | ceiling).
    pub fn seeded(current: u32) -> Self {
        Self {
            current: current.max(1),
            best_goodput: 0.0,
        }
    }

    /// The live fleet size (already `>= 1`).
    pub fn current(&self) -> u32 {
        self.current
    }

    /// Step one evidence window. `window_bytes` moved (server-confirmed),
    /// `window_secs` elapsed, `calls` completed calls of which `errors`
    /// were counted retries/reattaches, `ceiling` =
    /// `min(MAX_WORKERS, declared)` for the job. Returns the change to
    /// journal, or `None` to hold.
    pub fn step(
        &mut self,
        window_bytes: u64,
        window_secs: f64,
        calls: u32,
        errors: u32,
        ceiling: u32,
    ) -> Option<FleetChange> {
        let ceiling = ceiling.max(1);
        let goodput = if window_secs > 0.0 {
            window_bytes as f64 / window_secs
        } else {
            0.0
        };
        // Error signal FIRST — errors outrank goodput (a drowning bearer
        // can still show aggregate through retries' waste). Gated by call
        // count, not bytes: an error storm with nothing confirmed is
        // exactly the verdict that must land.
        if calls >= FEEDBACK_MIN_CALLS {
            let err_rate = f64::from(errors) / f64::from(calls);
            if err_rate > ERR_RATE_DECREASE {
                let to = ((self.current as f64 * DECREASE_FACTOR).floor() as u32)
                    .max(1)
                    .min(ceiling);
                return self.decide(to, FleetReason::Errors, goodput);
            }
        }
        // Goodput arms: gated by bytes AND window length — a window that
        // moved too little to judge HOLDS (never guesses).
        if window_bytes < FEEDBACK_MIN_BYTES || window_secs < FEEDBACK_MIN_SECS {
            return None;
        }
        // Baseline: the first clean window is the reference, not a signal.
        if self.best_goodput <= 0.0 {
            self.best_goodput = goodput;
            return None;
        }
        if goodput > self.best_goodput * GROWTH_RATIO {
            let to = self.current.saturating_add(GROWTH_STEP).min(ceiling);
            self.best_goodput = goodput;
            return self.decide(to, FleetReason::Growth, goodput);
        }
        if goodput < self.best_goodput * OVERSHOOT_RATIO {
            let to = self.current.saturating_sub(1).max(1);
            return self.decide(to, FleetReason::Overshoot, goodput);
        }
        // Plateau: between 0.9x and 1.08x of the best — the bearer is full
        // at this size. Hold (more streams would only contend).
        None
    }

    /// Commit and return the change — `None` (a silent hold) when the
    /// arithmetic lands on the same size.
    fn decide(&mut self, to: u32, reason: FleetReason, goodput: f64) -> Option<FleetChange> {
        if to == self.current {
            return None;
        }
        let change = FleetChange {
            from: self.current,
            to,
            reason,
            goodput_bps: goodput,
        };
        self.current = to;
        Some(change)
    }
}
