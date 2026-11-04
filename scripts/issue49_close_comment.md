## CLOSED — the full loop, landed at `6d55fd4` (suite 160 → 170)

**RED first** (`scripts/issue48_red_evidence.md`): the drive fleet was
`pending.min(MAX_WORKERS).min(declared)` — a constant policy. No growth path existed
anywhere: eviction/resplit/frozen-rule only shrink, so a job planned at the owner's
3 KB/s moment stayed small forever after the link recovered 54×.

**The design evolved mid-loop on one architecture finding** (worth recording): workers
LOOP until the pool empties, so a clean job is ONE drive round — the issue's
round-granularity AIMD would never have ticked. The landed design is therefore a LIVE
waterline:

- **Seed** (journal-first, ADR-002): last `fleet_adjusted` | measured
  `recommended_fleet` (#48) | ceiling — clamped to what the job can use (pending count;
  a 3-piece job carries no 32-wide waterline).
- **Adapt**: a driver-side ticker folds 1 s completed-call evidence windows into the
  pure `AdaptiveFleet::step` (new module `fleet.rs`, every arm unit-tested):
  ×0.7 on err_rate > 0.3 (error verdicts need ≥ 4 calls — an error storm with zero
  confirmed bytes must still decide), +2 while goodput clears best × 1.08, −1 on a
  >10% regression, hard hold at the plateau, feedback gates (≥ 64 KiB AND ≥ 0.5 s for
  goodput verdicts).
- **Enforce**: workers above the waterline PARK at their lease boundary — watch-driven,
  250 ms bounded poll, exit when nothing is leasable — and wake on growth. Both
  directions bind MID-round.
- **Durable**: every decision journaled `fleet_adjusted {from, to, reason, goodput_bps}`
  at round end; a fresh manager re-derives the seed (no process memory).

**GREEN (manager-level, deterministic):**
- `scripted_errors_decrease_the_fleet_and_journal_the_reason` — 8 scripted transients →
  journaled `{from: 8, to: 5, reason: "errors"}` while the job still completes; the first
  wave provably ran 8 concurrent workers.
- `journaled_link_profile_caps_the_first_round` — the measured seed caps the first wave
  at exactly 2 and no wave ever exceeds it (workers 2..7 park BEFORE leasing).
- `last_fleet_adjusted_seeds_a_fresh_manager` — the journal decision survives a restart.

**Two drill bugs the drills caught in their own harness** (recorded per the postmortem
rule): a `fetch_sub` underflow in the test double (0 → u32::MAX = infinite errors), and
a parked-worker hang (the park arm never re-checked the pool; `join_all` waited forever).

**Honest notes:** RTT-inflation watch (bufferbloat) is not implementable yet — adapters
expose no per-request latency; error rate + plateau carry the decrease signal (future
delta). Window aliasing on sub-second mock calls is self-correcting via the overshoot
arm (documented in `fleet.rs`). Suite 170/170, gates clean, committed-tree gate GREEN.
