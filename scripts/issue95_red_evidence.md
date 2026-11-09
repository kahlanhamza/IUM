# Issue #95 — RED receipts (the resource-hygiene bar)

Parent tree: `5374a00` (the v0.2.0 STATUS closer). Captured 2026-11-09. One section per lane; GREEN receipts land in the issue comments, not here.

## R1 — the soak's fd pin

- **Absence receipt**: `git show 5374a00:ium-core/tests/soak_bounds.rs | grep -c fd` → **0** — the soak pinned journal/WAL/RSS only (the #34 record). `load_100.rs` already pinned fd (< 400 for 100 jobs, #66 lane 5); the hours-shaped leg did not.
- A module-based RED (`mod common;`) was attempted first and **resolved against the existing fixture module** (`tests/common/mod.rs` serves the ffmpeg real-data drills) — test passed, no failure to record. Superseded honestly: the lane follows the file-local helper style the suite already uses for `rss_kb` (duplicated deliberately; a shared fixture module would drag fixture dead-code into every consumer).
- **The canary caught TWO measurement flaws pre-landing** — the loop working as designed: draft one pinned the process fd *baseline* and failed under cargo's parallel test execution (`descriptors did not return after close: 10 vs baseline 6` — the soak's own sqlite fds were legitimately open); draft two pinned the reference window and the FULL-SUITE gate caught the soak's startup churn the standalone run never saw (`fd_probe_detects_real_descriptor_growth ... FAILED`). Final shape: symmetric ±1-churn deltas around the narrow read windows — the detector must SEE ~5 opens AND their closure; everything else is sibling noise. Both lessons live in the test's comment.
- **Detector-alive canary** (`fd_probe_detects_real_descriptor_growth`): 5 real handles → detector sees ≥ +5; close → returns to reference ±1. The pin cannot vacuously pass.
- GREEN (local, Linux): `SOAK-BOUNDS {"events":16391,...,"fd_delta":0,"bounds":{...,"fd":8}}` — the soak opens and retains ZERO descriptors across 16 k events; bound pinned at 8 (generous against sqlite's WAL/SHM pair, tight against per-event retention).

## R2 — the shell idle + RSS pin

- **Absence receipt**: at `5374a00`, `.github/workflows/ci.yml` contains **0** references to resource/idle assertions (grep = 0) and `scripts/resource_pin.sh` does not exist — the app-e2e job drove the shell and measured nothing about its own footprint.
- **Detector proof** (`--self-test`, runnable everywhere, no WebKit): synthetic busy child = **+2000 ms** vs sleeping child = **+0 ms** over a 2 s window — the sampler separates burn from idle; the live pin cannot vacuously pass.
- Live mode boots the real shell under xvfb (DMABUF disabled, the #35 lesson) and pins a 60 s quiet window: tree CPU-time delta ≤ 5000 ms (a 100% polling loop burns ~60 000) and tree RSS ≤ 1.5 GiB (WebKitGTK baseline is a few hundred MiB). Provisional-generous by design; the CI receipt tightens them.

## R3 — the long-haul plateau pin

- **Absence receipt**: at `5374a00` the bin's ONLY "plateau" match is an S4 doc comment ("the frontier sits at 25%") — the longhaul leg printed `[longhaul][persist]` RSS telemetry (the #34 record) but **asserted nothing**, and stress.yml contained **0** references to longhaul: the overnight shape was never wired into the scheduled lane.
- **Landed**: the telemetry task now collects the (t, RSS) sample series plus the fd span (first/last across pause AND fresh-process replay); post-leg the bin computes the least-squares RSS slope (warmup first 25% dropped) and fails the leg past **24 MiB/hour** (a 1 KiB-per-tick leak = 720 KiB/hour: caught); fd drift > 4 fails the leg; short legs loud-skip the plateau, never silently.
- **Local proof (full arc, short leg)**: `EVIDENCE {"fd_span":{"first":21,"last":13,"delta":0,"bound":4},"plateau":{"skipped":true,"reason":"samples 8 < 40"},"replayed_and_resumed":true,"state":"Done"}` — byte-exact readback, journal replayable, s3-test GREEN against pinned SeaweedFS 4.51.
- **The ≥ 40-sample plateau computation is CI's proof**: the new `long_soak` stress job (default 55 min: 30 MiB @ 8 KiB/s + pause + fresh-process replay, timeout 150 min) rides the Saturday schedule and dispatch. Sandbox note: background processes die when the tool session ends here — two 10-minute local legs died one minute in (partial evidence file, no process); the long-leg proof belongs to CI's runner, where the job owns it.
- YAML restructure caught on read-back pre-commit: the first edit landed `long_soak` between the realworld job's steps, orphaning its evidence-upload step — fixed before any commit.

## R4 — the sanitizer lane

- **Absence receipt**: `git show 5374a00:.github/workflows/sanitize.yml` → **absent**; no workflow references sanitizers (grep = 0).
- **Landed**: `sanitize.yml` — weekly (Sundays 04:00 UTC; stress owns Saturdays) + dispatchable; ASan + LSan (nightly, `-Zsanitizer=address`, `detect_leaks=1`) over (1) the full ium-core suite (env-gated wire tests skip loud without servers), (2) the canonical S3 wire suite vs pinned SeaweedFS 4.51, (3) the TUS wire suite vs pinned tusd 2.10.1. NOT per-push (the CI-economics rule: full nightly recompile + ~2-3× runtime).
- Scope: the engine is the leak surface the loop owns — a system WebKit under ASan would drown the signal in foreign allocations; the shell's honesty is R2's job.
- Verification: YAML-lint green locally; the first dispatch run is the lane's real proof (foreign-allocation false-positive risk handled on the record if it fires).

## CI-caught fixes (round 1, runs 42541818712 / 42541837469)

- **R2 — the sampler read the wrong stat field**: the live pin failed with `tree RSS 1885448087439360KiB` (~1.1 petabytes): `/proc/<pid>/stat` field `[23]` is not the RSS page count (`[21]` is) — the CPU-only self-test sailed through because the RSS path had no proof. Fixed: `[21]` + the self-test now guards RSS sanity (a real process is 1 MiB–64 GiB). The same receipt delivered R2's first honest number: **cpu_delta 420 ms over the 60 s quiet window — the booted shell idles at ~0.7% of one core.**
- **R4 — proc-macros cannot carry sanitizers**: `error: building proc macro crate with sanitizers enabled is not supported` on `zerofrom-derive`. Fixed: `CARGO_BUILD_TARGET: x86_64-unknown-linux-gnu` at the job level — an explicit target keeps host proc-macros out of the sanitized compilation, and covers every cargo call inside the harness scripts.

## R3 dispatch round 1 (run 42541835145) — the long soak's first REAL find

The 55-minute honest rehearsal of the owner's KB/s uplink (8 KiB/s bearer, 4-way fleet) FAILED: `frontier=0` at t+3296 s, `chunk_evicted` cascade, attempts exhausted, `resumed leg ended Failed`. The journal + proxy receipts (artifact `long-soak-evidence`) pin the mechanism:

- **Eviction medians poisoned by the socket-buffer burst**: the `chunk_evicted` events carry `median_bps` of **461 169 / 266 615 / 72 091 B/s on an 8 192 B/s bearer** — physically impossible without each lease's FIRST window measuring the client's kernel-buffer drain (hundreds of KB at localhost speed) instead of the wire.
- **The honest trickle then sits below 0.4×median for 2 windows** → evicted → re-lease re-bursts and re-dies → attempt budget exhausted. The proxy's `client abandoned PUT mid-body` lines at 82–129 s with 0.5–1.5 MB moved match the 2-strike arithmetic exactly.
- **Harness layer**: every abandoned PUT's `VICTIM-WIRE` shows the FULL 8 243–8 277 B/s while its peers starve at honest `bps: 0.0` — the shared bucket granted 250 ms of tokens per take, and the lead relay's wake phase monopolized it (`take()` drains the whole grant and re-queues first).

Two fixes, one per layer:

1. **Engine (the product bug)**: `ThroughputWatch` now DISCARDS each lease's primer window — it measures the kernel socket-buffer drain, not the wire, on ANY link where part-time ≫ window (the owner's real uplink qualifies; no proxy required). The primer still seeds the accounting so window 2 measures the true steady-state delta. Unit test extended: primer discarded, window-2 delta exact, primer bursts cannot leak.
2. **Harness (the test double)**: the shared bucket grants ONE 20 ms poll-slice per take (`rate × 0.02`, was ×0.25) — no single connection can outrun a cycle; the wake rotation shares the bearer by construction.

Full core suite green after the engine change (46/46 binaries). End-to-end validation = the next `long_soak` dispatch (the 55-minute leg must reach `Done` with an honest plateau receipt).

## R3 dispatch round 2 (run 42550649711) + the local repro — the send path was the killer

The round-2 dispatch on the primer fix FAILED with the same terminal shape
(`resumed leg ended Failed`). The pushed tree could not say why; the
diagnostics commit (ef0d836, this lane) gives every future failure a name.
The local repro (30 MiB @ 8192 B/s, the diagnostic build) then named it in
ONE leg — the receipt chain:

```
transient: transport: error sending request ... ← client error (SendRequest)
  ← error writing a body to connection ← Connection timed out (os error 110)
```

**The un-paced send path**: hyper blasts the part body into the socket until
the kernel buffers fill (~1.1 MB in this box, `tcp_wmem` max 931 136 B), then
the write pends for the drain — 8–95 s at a stretch at 8 KiB/s (measured with
a bare-socket experiment) — and the TCP stack kills the stalled writer. The
engine retried the ENTIRE part from zero (S3 parts are atomic), 8 attempts
each, journal frozen, budget dead. Isolation experiment: a paced receiver
does NOT kill a bare sender — the kill needs the engine's exact writer
shape; the receipt, not the theory, names it.

**The fix (round 2, three layers):**

1. **`PacedAsyncRead`** (adapter_s3.rs): the body reads pace to the MEASURED
   wire — a debt-carrying token bucket (2 s burst ceiling) over tokio's
   clock, so the socket never buffers deeper than the ceiling and the writer
   never pends longer than the wire itself. 0 = unpaced passthrough (no
   measurement → byte-identical behavior; fast links unchanged — the pacing
   engages only below a 4 MiB/s ceiling). Wired through the existing #57
   probe: `TransferAdapter::notify_link_bps` (default no-op — zero breakage)
   delivers `profile.single_bps` after a successful probe, and `run_job`
   re-notifies from the JOURNALED link_profile on every drive — the pace
   survives a pause/crash/restart. Virtual-time pins: wire conservation
   (3000 B @ 1000 B/s = ~2.0 s), passthrough, burst ceiling.
2. **The saturated-lease hold** (manager.rs): a lease whose body is FULLY
   emitted is not stalled — it is waiting for the confirm. S3 parts are
   atomic: an eviction there re-PUTs the whole span from zero (the leg's
   3 evictions = 22 MB of pure waste). The window doesn't vote and doesn't
   strike.
3. **`reconcile_parts` dedups a re-PUT duplicate**: SeaweedFS's ListParts
   listed BOTH copies of a re-PUT part (`expected 2, found 1; server listed
   6 part(s): (p1) (p1) (p2)...`). S3 replace semantics: the latest write IS
   the part; dedup by part number, judge contiguity on the deduped list.

**Local GREEN (full arc)**: 30 MiB @ 65536 B/s, `IUM_LINK_PROBE=1`,
`EVIDENCE {... "state":"Done", "mid_state":"done", "frontier_at_pause":
31457280, "fd_span":{"delta":0,"bound":4}, "rss_hwm_kb":14408,
"replayed_and_resumed":true, byte-exact readback}` — the paced leg holds
`VICTIM-WIRE moved=7864320B in 120.0s (65536 B/s)` per part, zero
abandons. The long_soak stress job now sets `IUM_LINK_PROBE=1` (the owner's
shell posture — the leg models it) so the measurement and the pace engage.

### Round-2 addendum: the saturated-lease hold was REJECTED by CI

The first fix shape held the eviction accounting back while a lease's body
was "fully emitted" (progress.moved() >= span) — reasoning that the lease
was confirm-pending, not stalled. CI's own S3-hostile drill rejected it
within the hour: `s3hostile: no chunk_evicted in the journal — eviction did
NOT fire on the impaired link`. The drill's impaired connection absorbs its
whole body into kernel buffers, so "emitted" ≠ "on the wire" — deep buffers
make the hold treat a crawling lane as confirm-pending, and the eviction
contract (spec §4.2, aria2's connection re-establishment) is the DESIGN for
exactly that shape. REVERTED (this commit); the eviction semantics are
byte-identical to the baseline again. What stands: the pacing (paced links
never over-buffer, so their confirms land without the race the hold tried to
fix) and the reconcile dedup (any eviction re-PUT duplicates are now
reconciled honestly). The lane_floor drill passes on the reverted tree.
