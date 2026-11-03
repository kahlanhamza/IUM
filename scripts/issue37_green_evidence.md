# #37 GREEN — CI red lanes closed on local proof, all four wire lanes

## 1. Windows compile/clippy — the rlimit drill is Unix-scoped by construction

`ium-core/tests/enospc_rlimit_drill.rs` is now `#![cfg(unix)]` at file level: the
drill's whole mechanism (`RLIMIT_FSIZE`, `SIGXFSZ`) does not exist on Windows, so
the windows lane compiles the target to an empty binary — no `libc::rlimit`
E0422/E0425, no unused-import warnings, nothing to `#[allow]` away. The drill
itself still runs on the linux + macOS lanes (its own process, per the file doc).

Local gate at the fix commit:

```
cargo fmt --all -- --check                                  → clean
cargo clippy -p ium-core --all-targets -- -D warnings       → Finished, 0 warnings
cargo test -p ium-core                                      → all targets 0 failed
```

(The definitive windows-lane proof is CI itself: the fix commit's
`core (windows-latest)` job — clippy + tests + docs — recorded in the run for
this push.)

## 2. @tus/server diversity lane — the flake's two faces, both closed

**Face 1 — `tus_bogus_session_maps_to_session_lost` (CI run #63):** the stale/
unknown-session PATCH is early-rejected by @tus/server, which writes the 404 and
closes the connection WITHOUT consuming the body — the client, still streaming
6 MiB, sees a connection reset, not the status. The adapter never declares from
the reset alone: after a PATCH transport failure it HEADs the session, and the
SERVER's answer decides — `404/410` → `SessionLost` (→ Reattach), success or
HEAD-transport-failure → `Transient` (retries stay counted). Verify-before-declare,
for error paths too. tusd lane behavior unchanged (real 404 → `SessionLost`
directly).

**Face 2 — `tus_offset_conflict_realigns_from_server_offset` (reproduced locally,
1-in-3 lane runs):** the exact waste assert came up 8 KiB short
(`moved=3211264 wasted=1105920`, expected `moved − wasted == 2097152`). Root
cause: the rejected attempt's last chunk landed in `moved()` AFTER the instant
waste snapshot — the count is produced on hyper's connection task and the
snapshot was a same-instant read from the caller's task. Fix is structural:

* `ProgressReporter` counters are `SeqCst` — cross-task accounting reads are
  coherent the moment a response round-trip has passed (shared with the S3
  adapter, which counts wire bytes the same way);
* the waste classification is now SETTLED ARITHMETIC instead of a racy snapshot:
  after the realign PATCH completes, everything the call pushed beyond the
  re-PATCH's server-acked tail (`pushed − tail`, `tail = realigned −
  server_offset`) is BY CONSTRUCTION the rejected attempt's flush — persisted
  nowhere, exactly waste, whenever its count landed. `moved() − wasted()` stays
  an exact invariant, not an eventually-consistent one.

## 3. Lane proof at the fix commit

| lane | suite | result |
|---|---|---|
| tusd 2.10.1 (canonical) | `scripts/tus-test.sh` × 4 | 12/12 each, GREEN |
| @tus/server 2.4.7 (diversity) | `scripts/tus-test.sh --server tus-node` × 6 | 12/12 each, GREEN (was 1-in-3 RED) |
| SeaweedFS 4.51 (canonical S3) | `scripts/s3-test.sh` | 23/23, GREEN |
| rclone serve s3 1.75.2 (diversity) | `scripts/s3-test.sh --server rclone` | 23/23, GREEN |

The dead-server negative path is exercised too: with NO server listening, the
PATCH transport-fails, the HEAD probe also transport-fails, and the error stays
`Transient("PATCH failed: …")` — a genuinely down server remains retryable, the
session is not declared lost from a socket's mood.

## 4. Scope discipline

* `adapter_tus.rs`: the transport-failure HEAD probe (patch), the settled waste
  reckoning (conflict arm), module-doc wire-fact row (#33/#37). No other
  behavior touched.
* `adapter.rs`: `ProgressReporter` ordering only.
* `docs/08_Boundary_Register.md`: the #37 wire-fact row on
  `adapter_tus ↔ server wire`.
* Unrelated observations (none) would get their own issue per the loop.
