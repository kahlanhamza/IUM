# #34 GREEN — all three acceptance criteria, with measured numbers

## 1. ENOSPC drill — write failure is LOUD, honest, recoverable

`tests/enospc_drill.rs` (injected SQLITE_FULL, deterministic, all platforms) and
`tests/enospc_rlimit_drill.rs` (REAL EFBIG: RLIMIT_FSIZE + SIGXFSZ ignored, own
process) — both GREEN after the fix:

- mid-transfer journal-write failure → `Paused` + `journal_write_failed` atomically
  journaled (one transaction), loud `EngineError::Journal` to the caller, no false
  Done/Failed, no corruption;
- with the disk STILL full (the rlimit leg): the park itself cannot land — the error
  stays loud, the row keeps its truthful last state, and recovery after freeing space
  completes byte-exact (`verify.ok` over the whole pattern);
- the half-transition hole is closed: `transition()` now commits state + event in ONE
  transaction (`SessionJournal::set_job_state_event`) — the RED "Running with NO state
  event" is impossible by construction;
- `Finishing`-boundary failures deliberately stay `Finishing` (Paused is not a legal
  arrow; Failed would be dishonest while the verify gate may already have completed the
  object) — resume re-enters the gate, Complete idempotency per #24.

## 2. Journal size + manager RSS pinned under a many-event soak

`tests/soak_bounds.rs` drives the REAL manager path — lease → upload → partial
server-acked confirm → re-lease, the exact wire shape of a B/s-class link — through
**16,391 journal events** (64 MiB in 4 KiB segments, 8 workers). Measured this session:

```
SOAK-BOUNDS {"events":16391,"journal_bytes":2502656,"wal_bytes_after_checkpoint":0,
             "rss_delta_kb":5644,"elapsed_secs":22.1}
```

- **journal: 2,502,656 B ≈ 2.4 MB → ~153 B/event, linear** (bound pinned: 32 MB);
- **WAL after `checkpoint_wal(TRUNCATE)`: 0 B** — truth lives in the main db (bound 4096 B);
- **manager RSS delta: 5,644 KiB** for the whole soak (bound 256 MiB) — orchestration
  state is bounded by the PLAN, not the event count; events are never held in memory.

Extrapolation for the owner's overnight leg: 30 MB @ 512 B/s ≈ 17 h produces a few
thousand events → a few MB of journal. Compaction/VACUUM is NOT needed — stays out of
scope per the issue (own issue if a future soak shows otherwise).

## 3. Long-horizon B/s leg — harness shipped, proven live, overnight config documented

`realworld_stress longhaul` + `scripts/longhaul.sh` + `s3-test.sh --with-longhaul FILE RATE SECS`:

- uniform paced lane (throttle proxy `--victim-part all`), `[longhaul][persist]`
  telemetry every 5 s (frontier / journal bytes / RSS), artifact in `rea-lab/longhaul/`;
- honest TIMEBOX: pause (never cancel — the leg stays resumable), landing at the next
  lease boundary (S3 parts are atomic mid-flight — part-scale physics, documented);
- FRESH manager over the same journal (the overnight "next session") replays and
  resumes to Done; verify-before-declare held; direct read-back sha256 byte-exact.

**Live vs pinned SeaweedFS 4.51** (12.9 MiB real h264+aac film through the paced lane):

```
EVIDENCE {"scenario":"longhaul","mid_state":"paused","pause_landed_s":176.4,
          "frontier_at_pause":0,"journal_bytes_at_pause":222296,"rss_hwm_kb":12860,
          "state_events":6,"replayed_and_resumed":true,"state":"Done",...}
EVIDENCE {"scenario":"longhaul-readback","sha256_ok":true,...}
```

The pause landed at 176 s (the next lease boundary at 32 KiB/s — real physics, not a
poll race); the replay resumed byte-exact. The first live run's own artifact caught a
harness bug (endpoint bypassed the proxy — frontier jumped to full, no UNIFORM lines);
fixed and re-run green — the telemetry is doing its job.

**The 17-hour configuration** (30 MB @ 512 B/s) is ready to launch in the free cloud
with the same command sized up (`--seconds 61200`, `IUM_LONGHAUL_PAUSE_SECS` past one
part's wire time, `IUM_LONGHAUL_RESUME_SECS` for the resume window). In-session it is
honestly **timebox-cancel**: this session cannot hold 17 h, and the criterion allows
"Done or honest timebox-cancel" — the leg machinery, telemetry, artifact, replay and
byte-exactness are what shipped and are proven live above.

## Gates

```
cargo fmt --all -- --check                 GREEN
cargo clippy -p ium-core --all-targets -- -D warnings   GREEN
cargo test -p ium-core                     150/150 across 23 targets, 0 failed
```
