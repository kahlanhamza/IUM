# Sprint 0 — Engine Record (audit findings & fixes)

> Scope decision for this sprint: **engine first, no UI work.** The Tauri shell
> exists as a scaffold and is parked until the ground it stands on is solid.
> Every item below is backed by a test that fails without the fix.

## What the engine gained this sprint

### 1. Journal hardening (`journal.rs`)
- **Versioned migrations** via `PRAGMA user_version`, each applied inside a
  transaction — a crash mid-migration rolls back whole; reopening replays only
  what never landed. Test: `migrations_are_versioned_and_reopen_safe`.
- **Atomic job creation** — the job row and its chunk plan commit together
  (`unchecked_transaction`); a crash between them leaves neither.
  Test: `duplicate_job_is_rejected` (dup-check inside the same tx).
- **Dead writes fail loudly** — `set_job_state` / `update_chunk` /
  `replace_chunks` verify affected rows and return `UnknownJob` /
  `UnknownChunk` instead of silently writing nowhere.
  Test: `writes_to_missing_rows_fail_loudly`.

### 2. Supervisor correctness (`manager.rs`)
- **The state machine validates against the journal, not caller arguments.**
  Previously `transition(from, to)` trusted the caller's `from` — decorative
  safety. Now the current state is read from the journal and the transition
  `(current → to)` is validated before it is journaled.
- **Partial confirms resume instead of vanishing.** A chunk confirmed only
  halfway went back into the pool as `Leased`-with-progress — a state
  `lease()` never hands out — so the supervisor could reach `Finishing` with
  missing bytes and declare `Done`. Now partial confirms return the chunk to
  `Pending` keeping confirmed bytes; the next lease resumes at
  `resume_from`. Test: `partial_confirm_resumes_from_confirmed_offset`
  (asserts the exact second-lease offset and that no byte was re-sent).
- **Retry attempts are counted per chunk.** The pool used to be reloaded from
  the journal every iteration, resetting `attempt` to 1 forever — a hostile
  endpoint could wedge the supervisor in an infinite retry loop. Now attempts
  are tracked per drive; exhaustion reaches `Failed` deterministically.
  Test: `transient_failures_are_retried_then_exhausted` (asserts exactly
  `max_attempts + 1` upload calls and the attempt sequence 1..4).
- **Crash recovery from any journaled state** — `Running`/`Finishing` on
  entry continue where the journal says; stale `Leased` rows left by a dead
  driver are reclaimed (confirmed bytes kept). Tests:
  `crash_mid_running_recovers_without_reuploading_confirmed_bytes`,
  `stale_lease_from_crashed_driver_is_reclaimed`.
- **Single-driver guard** — two concurrent `run_job` calls on one job is an
  error, not a lease race. RAII guard releases the slot even on panic.
  Test: `double_run_is_rejected_not_raced`.
- **`Retry-After` is honored** — the adapter's `Throttled { retry_after_ms }`
  now reaches the policy (it was silently dropped before).
- **One-shot honesty in the chunk plan** — non-resumable endpoints are
  planned as exactly one chunk; chunking a stream that cannot resume would
  imply a capability the protocol lacks.
- **Verify consistency gate** — reaching `Finishing` with a not-fully-
  confirmed chunk journals `verify_inconsistent` and fails instead of lying.

### 3. Chunk pool soundness (`chunk.rs`) — found by the invariant fuzz
- **`resplit_pending` invalidated in-memory leases.** It renumbers chunks;
  leases keyed by old indexes then "protected" unrelated chunks → the pool
  could wedge (non-done chunks that no `lease()` can ever return). Found by
  `fuzz_random_operations_never_break_invariants` (25 rounds × 400 random ops
  + drain-to-completion). Fix: a resplit retires outstanding leases; leased
  chunks return to `Pending` keeping confirmed bytes.

### 4. Adapter contract sharpened (`adapter.rs`)
- **Adapters read the disk themselves.** `upload_chunk(spec, lease)` no longer
  receives an in-memory `Arc<Vec<u8>>` — the whole-file-in-RAM shortcut is
  gone. The manager never buffers file bytes.
- **`ChunkConfirm.bytes_confirmed` is segment-relative** (what THIS call got
  acked for `[resume_from, end)`); the manager composes it with prior
  confirmations. The absolute/segment mismatch was another infinite-lease bug
  caught by the resume test hanging.

### 5. Real network transport (`adapter_http.rs`)
- `HttpOneShotAdapter`: streaming `PUT` straight from disk
  (`tokio_util::codec::FramedRead` + `AsyncReadExt::take`, bounded by the
  lease), pure-Rust TLS (`reqwest` + rustls — no OpenSSL linkage), error
  taxonomy mapping (429→`Throttled`, 403/410→`ExpiredSession`, 5xx→
  `Transient`), ETag capture as the reattach token, and a **HEAD
  verify-before-declare gate**.
- **Trap found by the loopback test:** `resp.content_length()` reports the
  body size hint, and hyper correctly yields `Some(0)` for HEAD responses (a
  HEAD body is empty by definition) — every honest server would look like a
  lying one. Fix: read the `Content-Length` header itself.
  Test: `verify_gate_catches_a_server_that_lies_about_bytes` proves a real
  liar is caught, and the happy path proves an honest server passes.

### 6. End-to-end proof over real TCP (`tests/http_one_shot.rs`)
A hand-rolled loopback server (request-head parsing, **chunked
transfer-encoding decode**, per-behavior fault injection: 200+ETag, 429+
`Retry-After`, 500, lying HEAD):
- happy path: exact file bytes arrive over the wire exactly once, ETag
  journaled, state path `created→probing→running→finishing→done` verified
  from the event log;
- 500-exhaustion: exactly `max_attempts+1` PUTs, zero claimed bytes kept;
- 429 + `Retry-After: 0`: throttled once, retried once, done — and a
  rejected PUT's bytes are NOT committed server-side (the verify gate caught
  the naive server model that committed them);
- lying server: HEAD reports fewer bytes than sent → `Failed` with
  `verify_failed` naming the discrepancy. The gate works.

## CI
- The workflow YAML was **broken on `main`** (`branches: ain]` — a truncated
  list) — meaning CI never ran. Fixed; engine matrix now: fmt (linux) +
  clippy `-D warnings` + tests on **ubuntu/macos/windows**.
- The desktop shell is deliberately not compiled in CI yet (scaffold
  manifests are validated); it gets its own matrix when work resumes there.

## Numbers
- `cargo fmt --check`: clean · `cargo clippy -p ium-core --all-targets -- -D warnings`: clean
- `cargo test -p ium-core`: **45 passed** (28 unit · 13 integration · 4 HTTP end-to-end)
- Engine deps: tokio, reqwest (rustls), rusqlite (bundled), tokio-util, serde, thiserror, tracing, async-trait, getrandom
