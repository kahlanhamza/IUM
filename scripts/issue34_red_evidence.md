# #34 RED evidence — the ENOSPC holes, failing at current HEAD

Two new drills fail at HEAD (`8da0b1b`), reproducible with:

```bash
cargo test -p ium-core --offline --test enospc_drill
```

## RED 1 — mid-transfer ENOSPC dangles the job at Running

```
running 2 tests
test half_transition_leaves_no_state_without_its_event ... FAILED
test enospc_mid_transfer_parks_paused_loud_and_resumes_byte_exact ... FAILED

---- enospc_mid_transfer_parks_paused_loud_and_resumes_byte_exact stdout ----
assertion `left == right` failed: mid-transfer ENOSPC must park the job Paused, honestly
  left: Running
 right: Paused
```

`run_job` DOES return a loud `EngineError::Journal` (that part holds), but the
job ROW stays `Running` with nothing driving it and no journaled honest end —
the acceptance criterion "job ends Paused/Failed honestly" is not met. The
error is loud only to the caller; the journal's own record is a lie of
omission.

## RED 2 — the half-transition: a state row without its event

```
---- half_transition_leaves_no_state_without_its_event stdout ----
journal says Running with NO state event — half-transition corruption
```

`transition()` writes `set_job_state` + `append_event` as two autocommit
statements. With the drill hook failing exactly the second write (the state
event append — the disk filling between them), the journal commits the state
row and loses the event: a state with no trail. That is the "silent
corruption" shape ENOSPC can produce today.

## The real-OS anchor (green at HEAD, stays green through the fix)

`tests/enospc_rlimit_drill.rs` (own process): `RLIMIT_FSIZE` + SIGXFSZ ignored
— writes past the cap fail EFBIG, SQLite surfaces SystemIoFailure. It pins the
REAL error shape the injected drill replicates: loud error, truthful state row
(Running is the honest "drive died mid-round"), state-implies-event holds even
there, no false Done/Failed, recovery byte-exact after space is freed.

```
test real_disk_full_is_loud_truthful_and_recoverable ... ok
```

## What the fix must do (design note §4)

1. `SessionJournal::set_job_state_event` — UPDATE + event INSERT in ONE
   transaction; `transition()` switches to it. Both-or-neither.
2. `JobManager`: on `EngineError::Journal` escaping `drive_running`/`finish`,
   best-effort atomic park to `Paused` + `journal_write_failed` (when the
   current state is `Running`; other states already recover truthfully via
   crash-recovery resume), original error still returned. If the park itself
   cannot land (space still full), log loudly and return the original error —
   loudness moves to the error channel, the row stays truthful.
3. The injected hook (`SessionJournal::drill_fail_writes`, already in this
   branch as test infrastructure) stays: it is the deterministic root-free
   stand-in; the rlimit drill proves the shape matches real EFBIG.
