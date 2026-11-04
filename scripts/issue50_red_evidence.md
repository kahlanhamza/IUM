# RED evidence — tus concatenation lane (issue #50)

**Command:** `cargo test -p ium-core --test tus_concat` (at HEAD, before any
implementation — the new suite `tests/tus_concat.rs` drives the adapter API
the lane requires).

**Output:**

```
error[E0432]: unresolved imports `ium_core::adapter_tus::encode_concat_token`, `ium_core::adapter_tus::concat_partition`, `ium_core::adapter_tus::parse_session_token`, `ium_core::adapter_tus::SessionToken`
error[E0599]: no method named `probe_fleet` found for struct `TusAdapter` in the current scope
error[E0599]: no method named `finalize` found for struct `TusAdapter` in the current scope
error[E0282]: type annotations needed for `Vec<_>`
error: could not compile `ium-core` (test "tus_concat") due to 10 previous errors
EXIT: 101
```

**Interpretation:** the concatenation lane does not exist — no
`Upload-Concat` anywhere in the adapter (grep-clean), no discovery, no
partial sessions, no finalize. The suite's raw-HTTP server-vector legs are
written and ready; the adapter legs are the RED (compile-fail, the loop's
API-contract precedent — `verify_is_required`). Green comes with the
implementation: discovery journaled, N partials through the REAL drive
loop, final POST at the all-Confirmed boundary, verify on the final URL.
