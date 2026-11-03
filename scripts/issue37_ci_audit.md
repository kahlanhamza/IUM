## CI audit — runs #62–#69 (requested by the repo owner: main stayed red while new pushes kept landing)

| run | sha | failing job | classification |
| --- | --- | --- | --- |
| #62 | `8da0b1b` | core (macos) `cancelled` | concurrency phantom (superseded mid-run) — not a real failure |
| #63 | `12bb160` | core windows + ubuntu | **A**: `enospc_rlimit_drill` compile (unix-only `libc::rlimit` outside `#[cfg(unix)]`, windows) + **B**: `tus_bogus_session_maps_to_session_lost` vs tusd (transport reset read as `Transient`) |
| #64 | `8950cd8` | core ubuntu | **C**: `tus_offset_conflict_realigns_from_server_offset` FAILED once (8 KiB flush landing after `patch()` returned — the settled-arithmetic hole) |
| #65–#69 | `7969318`…`ddca814` | **app-e2e ×5** | **D**: deterministic design flaw, same assert every time: `live progress must move: first=0% second=0%` (`e2e_shell.rs:434`) |

### D — the app-e2e red is a test-design bug, not flakiness

Chain of facts:

1. The UI's percent is `Σ bytes_confirmed / source_size` (`logic.ts percentDone`) — server-confirmed bytes only, by the spec §2.3 honesty rule. Correct.
2. Leg A of the e2e never touches the adapter `<select>`, so it runs the registry default `http_oneshot` (first registered).
3. `HttpOneShotAdapter` plans **one chunk for the whole file** and journals `ChunkConfirm { bytes_confirmed: span }` only when the PUT completes (`adapter_http.rs`).
4. The recorder throttles at 2 MiB/s against a 24 MiB file ⇒ ~12 s of honest `0%`, then `100%`.
5. Therefore `p2 > p1` sampled 1.2 s apart is **unsatisfiable by construction** — 5/5 runs failed identically. No throttle tuning can fix it; the assertion contradicts the adapter's honest semantics.

### Fix direction (in progress)

Drive leg A through the **tus adapter** (registered, never exercised by the shell e2e):

* the loopback recorder gains a real TUS lane (`POST /files` creation → `Location`, `PATCH /files/<id>` → `204` + server-acked `Upload-Offset`, `HEAD` verify) — throttled like the PUT lane;
* per-PATCH server acks make `bytes_confirmed` advance **during** the transfer ⇒ the UI percent genuinely moves — the assertion becomes true by the product's own semantics instead of by luck;
* the progress assertion becomes a deadline poll (movement within 30 s), robust to chunk-boundary timing on a 2-core runner;
* leg B (cancel ⇒ no object, HEAD 404) stays one-shot: the honest "nothing committed" semantics belong to that family.

This also closes the coverage gap: the shell e2e never drove the resumable path at all.

### B/C status

Both faces are already addressed by the settled-arithmetic fix (`adapter_tus` three-way probe + `manager.rs` Conflict-arm waste reckoning + `ProgressReporter` SeqCst): core has been green on all three OSes and both server lanes for the 5 consecutive runs #65–#69. A local stress loop over the two tests backs this before this issue closes.
