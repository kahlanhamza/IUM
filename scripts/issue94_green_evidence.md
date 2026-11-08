GREEN evidence — #94 (the global speed cap)

Commit `451cc20` — **CI run 189 (42522928940): 8/8 success on the FIRST
verdict** (core ×3 OS, ium-app ×3 OS, fmt, app-e2e on WebKitGTK).

## RED → GREEN

- RED at HEAD `6a96623` (comment 6726298606): `unresolved import
  ium_core::rate` + no `set_rate_cap` anywhere in the engine — nothing
  could be polite about bandwidth.
- Design note first (comment 6726274873): the seam (pre-lease acquire),
  the eviction-safety argument, the honest persistence correction
  (policy knob, not journal truth — matching the set_policy family), the
  slicing requirement.
- GREEN: core **321/321** (313 + 6 integration pins + 2 bucket unit
  tests), clippy `-D warnings` clean, fmt clean, gate.sh GREEN on the
  committed tree; shell **163/163 across 19 files** (+6 speed pins), tsc
  strict.

## The pins caught two real defects pre-landing

1. **The single-shot deadlock**: a burst-clamped bucket can never satisfy
   an acquire larger than one cap-second — a 5 MiB lease under a 50 KB/s
   cap would wait forever. Fix: sliced grants (any lease completes in its
   fair `n/cap` time; the bucket never holds more than one burst).
2. **The over-sleeping shortfall**: sleeping the WHOLE deficit overshoots
   when the clamp bounds refill at one burst (19 s slept, 100 B granted).
   Fix: the wait targets min(remaining, burst).

Plus the lesson the RED measurements enforced: under tokio
`start_paused` virtual time, pacing must be MEASURED with tokio's clock —
`std::time::Instant` reads the real wall clock and reports microseconds.

## What shipped

- `ium-core/src/rate.rs`: the global Bucket (lazy refill, full-burst
  start, unlimited = no-op, hot `set_rate`).
- `manager.rs`: the acquire sits **before the lease** — nothing held
  during the wait (an abort/pause/resplit mid-wait re-runs the loop top:
  no stranded lease, no duplicated #47 guard), and the wait is OUTSIDE
  the eviction-tracked window (a capped wait can never be sampled as a
  stalled chunk — zero false evictions by construction). The paced size
  is `outstanding_target()` (the eventual lease is that size or smaller:
  honest-never-greedy).
- `supervisor.rs` → `set_rate_cap` command → `api.setRateCap`; Settings →
  **Speed** card (checkbox + KB/s field + the stated contract sentence);
  boot re-apply; SPEED_COPY brand-gated.
- Virtual-time pins: pacing exact (2000 B @ 100 B/s ⇒ 19 s), fleet SUM ≤
  cap with 4 concurrent workers, unlimited zero-overhead, hot swap,
  capped job byte-exact + eviction-safe end-to-end, uncapped prompt.

Closing completed.
