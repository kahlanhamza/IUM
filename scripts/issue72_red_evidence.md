# Issue #72 (Lane 5) — RED evidence (captured at HEAD 543c376, 2026-11-06)

## RED-1 — the supervisor concurrency limiter does not exist

`cargo test -p ium-core --test load_100` at current HEAD:

```
error[E0599]: no associated function or constant named `with_concurrency_limit` found for struct `Supervisor` in the current scope
error[E0599]: no associated function or constant named `with_concurrency_limit` found for struct `Supervisor` in the current scope
error[E0599]: no associated function or constant named `with_concurrency_limit` found for struct `Supervisor` in the current scope
error[E0599]: no associated function or constant named `with_concurrency_limit` found for struct `Supervisor` in the current scope
error[E0599]: no associated function or constant named `with_concurrency_limit` found for struct `Supervisor` in the current scope
error: could not compile `ium-core` (test "load_100") due to 5 previous errors
```

The supervisor spawns one drive task per `drive()` call with NO cap
(ium-core/src/supervisor.rs:103, W2) — a 100-file drag-drop intake is 100
simultaneous uploads. The tests in `ium-core/tests/load_100.rs` define the
contract: bounded admission, FIFO queue, honest `is_driving`, cancel/pause/
resume semantics through the limiter, and the 100-jobs load e2e.

## RED-2 — the installer-size budget gate does not exist

`python3 scripts/installer_size_gate.py --self-test` at current HEAD:

```
python3: can't open file '/home/z/my-project/scripts/installer_size_gate.py': [Errno 2] No such file or directory
```

No gate anywhere in release.yml fails an over-budget installer today (W1):
v0.1.1 measured NSIS 4.29 MB / MSI 6.02 MB / DMG 5.97 MB / deb 6.57 MB /
rpm 6.57 MB / AppImage 81.2 MB, and a 10× bloat would ship silently.

## Shell wiring (compile proof owed to CI)

The shell supervisor construction (ium-app/src-tauri/src/lib.rs:323) gains
the limit; the sandbox has no GTK — the standing honesty contract gives the
compile proof to CI's ium-app lane (same as #14/#61).
