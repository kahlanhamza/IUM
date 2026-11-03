## Component
ci / tooling

## Observed behavior
Two genuine red lanes on `main` at `12bb160` (run [#63](https://github.com/kahlanhamza/IUM/actions/runs/41676155580)); run #62's `core (macos-latest)` was cancelled (superseded), not a failure.

### A. `core (windows-latest)` — clippy/compile RED (deterministic, introduced by #34)
`ium-core/tests/enospc_rlimit_drill.rs` uses Unix-only libc APIs outside any `#[cfg(unix)]`:

```
error[E0422]: cannot find struct, variant or union type `rlimit` in crate `libc`
  --> ium-core\tests\enospc_rlimit_drill.rs:79:25
error[E0425]: cannot find function `setrlimit` in crate `libc`
  --> ium-core\tests\enospc_rlimit_drill.rs:83:33
error[E0425]: cannot find value `RLIMIT_FSIZE` in crate `libc`
  --> ium-core\tests\enospc_rlimit_drill.rs:83:49
error: unused import: `ium_core::state::JobState`        (line 34)
error: unused import: `SessionJournal`                   (line 35)
error: could not compile `ium-core` (test "enospc_rlimit_drill") due to 5 previous errors
```

The test fn itself is `#[cfg(unix)]`-gated, but `RlimitAdapter::arm_once()` (lines 79–83) and the module-level imports it needs are compiled on every target. The drill's mechanism (`RLIMIT_FSIZE` + `SIGXFSZ`) does not exist on Windows.

### B. `core (ubuntu-latest)` — TUS diversity lane flake (timing race, server-side)
`adapter_tus::tus_bogus_session_maps_to_session_lost` vs @tus/server 2.4.7:

```
thread 'tus_bogus_session_maps_to_session_lost' panicked at ium-core/tests/adapter_tus.rs:426:5:
404 on an unknown upload → SessionLost: transient: PATCH failed: error sending request for url (http://127.0.0.1:22388/files/does-not-exist)
test result: FAILED. 11 passed; 1 failed
```

Run #62 ran the **same test code** against the **same pinned server** and passed → not deterministic. Wire-fact: @tus/server rejects a PATCH on an unknown file **before consuming the request body** and closes the socket; the client is still streaming the 6 MiB body, so hyper may observe the connection reset (`error sending request`) instead of the 404 response. tusd (Go) drains/answers such that the 404 arrives; the canonical lane stays green. The adapter maps transport-level PATCH failures to `Transient` — honest for a real network blip, but it misreads this server-specific reset as retryable.

## Expected behavior — acceptance criteria
1. `cargo clippy -p ium-core --all-targets -- -D warnings` compiles cleanly on windows-latest — the rlimit drill is Unix-scoped by construction, not by accident of imports.
2. `tus_bogus_session_maps_to_session_lost` passes on BOTH lanes (tusd + @tus/server) deterministically — not by weakening the assertion, but because the adapter disambiguates: after a PATCH **transport** failure it HEADs the session URL; `404/410` → `SessionLost` (session really gone), HEAD success or HEAD transport failure → `Transient` (genuine blip, retries stay counted).
3. The adapter doc table gains the wire fact: "@tus/server answers an early-rejected PATCH by closing the connection (reset), not by writing the 404".
4. Full gates green locally; CI run on the fix commit green on all 3 OS × core + app.

## Failing test
- A: compile failure IS the RED (windows lane, run #63) — after the fix, `cargo check --target x86_64-pc-windows-gnu`-equivalent proof is the CI lane itself; locally the gate is `cargo clippy --all-targets` on linux + the file being fully `#![cfg(unix)]`.
- B: `IUM_TUS_ENDPOINT=... IUM_TUS_SERVER=tus-node cargo test -p ium-core --test adapter_tus tus_bogus_session_maps_to_session_lost` — RED at current HEAD on CI (run #63 log above); with the fix the PATCH-transport-failure path HEADs first and maps to `SessionLost`.
