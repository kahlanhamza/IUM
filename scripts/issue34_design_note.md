# #34 design note — long horizon & resource honesty

## 1. Entities touched (boundary register §2)

`journal`, `manager`, `harness` — exactly the register line this issue was filed
against. No adapter code changes; the wire seams (#32/#33) are untouched.

## 2. Edges at risk (§3 rows)

1. **journal ↔ everything** — the new failure POINT: ENOSPC while truth is being
   written. Two concrete holes probed at current HEAD:
   - **Half-transition**: `transition()` writes `set_job_state` + `append_event`
     as two autocommit statements. A disk-full landing between them commits the
     state row and loses the event — a journal that says "running" with no trail
     of how it got there. That is the "silent corruption" shape.
   - **Dangling Running**: an ENOSPC inside the worker loop (`apply_confirm`,
     lease `update_chunk`, `chunk_error` append) propagates `EngineError::Journal`
     to the caller — loud — but the job ROW stays `Running` with nothing driving
     it and no journaled honest end. The issue's criterion ("job ends
     Paused/Failed honestly") is not met.
2. **manager ↔ chunk pool** — resume after disk-free: the parked job's stale
   `Leased` rows must reclaim to `Pending` keeping confirmed bytes, and the
   resumed drive must complete byte-exact (pinned by the new drill).
3. **harness/proxy ↔ reality** — the hours-scale leg: same ladder machinery
   (#28 `s6worse`) with persistence telemetry (frontier, journal size, RSS,
   elapsed) and a wall-clock timebox that ends the job HONESTLY (pause, not
   cancel) so the journal stays replayable and resumable afterwards.

## 3. Failure semantics being pinned (the contract)

- **No silent corruption is possible today** — every write path checks
  rows-affected / propagates `rusqlite::Error` — but "loud" must ALSO mean the
  journal ends in a state a user can act on:
  - mid-transfer (`Running`) journal-write failure → journal `Paused` + ONE
    `journal_write_failed` event carrying the error, atomic with the state flip;
    caller still gets the loud error. Resume after freeing space → byte-exact Done.
  - if even the honest-park cannot be written (space still full), the error is
    logged loudly and returned; the state stays `Running` (truthful: the drive
    died mid-round) and crash-recovery resume still recovers it. No lie is
    writable to a full disk; loudness moves to the error channel.
  - `Finishing`-boundary failure → state stays `Finishing` (Paused is not a legal
    arrow from it, and Failed would be dishonest while the verify gate may
    already have completed the object server-side); resume re-enters the gate
    (Complete idempotency pinned by #24).
  - Done remains unreachable without a verified object (verify-before-declare,
    ADR-004) — the drill asserts no `done`/`verified` events after ENOSPC.

## 4. Implementation

1. **Atomic transitions** — new journal method `set_job_state_event(id, state,
   event_kind, payload)`: UPDATE (rows-affected-checked) + event INSERT inside
   ONE `unchecked_transaction`. `transition()` switches to it. A transition now
   lands both-or-neither; the half-transition hole dies.
2. **Drill hook (test-only, documented)** — `SessionJournal::drill_fail_writes(skip,
   fail)`: after `skip` write calls, inject `fail` SQLITE_FULL-shaped failures
   (`JournalError::Sqlite(SqliteFailure(SystemIoFailure))`). Makes disk-full
   deterministic without root; a byte-for-byte replica of the real error.
3. **Honest park** — `JobManager::pause_on_journal_failure`: on
   `EngineError::Journal` escaping `drive_running`/`finish`, if the journal state
   is `Running`, journal `journal_write_failed` + `Paused` atomically (best
   effort — swallow only the park's own failure, loudly logged), then return the
   ORIGINAL error.
4. **RLIMIT_FSIZE ENOSPC leg (unix, always-on)** — `SIGXFSZ` ignored + soft
   RLIMIT_FSIZE armed mid-transfer by the mock adapter on call N: writes beyond
   the cap return EFBIG, SQLite surfaces SQLITE_IOERR/SQLITE_FULL — the same
   `JournalError::Sqlite` the loopback-image drill produces on sudo runners (the
   env-gated `IUM_ENOSPC_LOOPBACK=1` image drill stays available for CI; loud
   skip without sudo).
5. **Soak bounds** — many-event soak through the REAL manager path (partial
   confirms ≈ tens of thousands of journal writes): pinned upper bounds on
   journal size after `wal_checkpoint(TRUNCATE)`, wal size after truncate, and
   Linux RSS delta; measured numbers recorded on this issue. Compaction/VACUUM
   stays out of scope (issue says: own issue if the soak shows need).
6. **Longhaul leg** — `scripts/longhaul.sh` + a persistence-telemetry harness
   run: B/s-choked lane, `IUM_LONGHAUL_MAX_SECS` timebox, periodic
   `[longhaul][persist]` lines (frontier, journal bytes, RSS, elapsed), honest
   pause at the timebox, fresh-process reopen proving journal replay + state
   truth. In-session: a short leg proves the harness live; the 30 MB @ 512 B/s
   ≈ 17 h overnight configuration is documented for the free cloud and recorded
   here as an honest timebox-cancel if the session cannot hold it.

## 5. Pinning tests (per edge, named)

- `tests/enospc_drill.rs::enospc_mid_transfer_parks_paused_loud_and_resumes_byte_exact`
  (unix rlimit leg; real OS-level EFBIG mid-drive → Paused + event + loud Err +
  no false Done + resume byte-exact). RED at HEAD: state is dangling `Running`.
- `tests/enospc_drill.rs::half_transition_is_atomic` (drill hook: fail exactly
  the event INSERT inside `transition()` → journal must be unchanged). RED at
  HEAD: state commits without its event.
- `tests/enospc_drill.rs::park_failure_still_returns_loud_error` (hook fails
  every write including the park → loud Err, state truthful-`Running`, no panic).
- `tests/soak_bounds.rs::many_event_soak_bounds_journal_and_rss` (bounds above).
- Longhaul: the `[longhaul][persist]` telemetry lines + reopen-verify in the run
  artifact (`rea-lab/longhaul/evidence-<ts>.md`).
- Existing pins untouched: `finishing_cancel.rs` (#26), `verify_repair.rs`
  (#24), stale-lease reclaim (integration), `slow_connection.rs` (#18).
