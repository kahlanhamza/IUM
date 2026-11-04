# Issue #51 design note — Multi-path bonding (M8#4)

> To be posted on the issue when the token is re-pasted (10th recycle wiped
> `.secrets/` again). Wire facts below were rehearsed LIVE on this sandbox
> (`scripts/wirefacts51/`, run 2026-11-05) before any implementation — the
> wire-facts-first discipline.

## Wire facts (rehearsed, not assumed)

| # | Fact | Consequence |
|---|------|-------------|
| W1 | `reqwest 0.12.28` exposes `ClientBuilder::local_address(IpAddr)` AND `ClientBuilder::interface(&str)` (SO_BINDTODEVICE on Linux, IP_BOUND_IF on macOS; cfg-gated to linux/macos/android/fuchsia/solaris/illumos — **Windows excluded**) | Both bind forms of the issue's `Path` abstraction map to native reqwest builder calls. **No custom connector, TLS intact.** Loud-skip on Windows = compile-time cfg, exactly the ADR-006 amendment pattern. |
| W2 | `SO_BINDTODEVICE('lo')` **succeeded as uid 1001** on this sandbox (P2 probe) | The strong form works here, but the capability MUST still be probed at runtime and per-OS declared — CI runners and user hosts differ. Never assume. |
| W3 | Source-binding 127.0.0.1 vs 127.0.0.2 → the server saw two distinct peers (P3) | The no-root drill rig is real: `lo` carries the whole 127.0.0.0/8; two source addresses = two honest paths. This is the CI/local drill rig and also the production rule for multi-NIC hosts (each NIC owns a source IP). |
| W4 | Dual-path aggregate = **2.28×** single-path (6738 → 15379 MiB/s, 64 MiB/path, P4) | The M8 acceptance "measured sum on a 2-path host" is demonstrated at the transport level; the managed-lane drill reproduces it through the engine. |

## Naming

The issue calls the abstraction `Path`; `std::path::Path` already shadows that
in every signature that touches the filesystem (`prober::spec_for(source:
&Path, ...)`). The module is `netpath`, the type `NetPath` — the mapping is
stated here so the issue's language and the code's names stay reconcilable.

## Design (maps 1:1 to the issue's numbered design)

1. **`NetPath`** — `enum { Default, SourceAddr(IpAddr), BindDevice(String) }`.
   `NetPath::client_builder()` returns a `reqwest::ClientBuilder` with the
   binding applied (W1): `SourceAddr → .local_address()`, `BindDevice →
   .interface()` under `cfg(any(target_os="linux", target_os="macos", ...))`;
   on other targets the variant is DECLARED but `client_builder()` fails
   loudly (`BindUnavailable { os }`) — adapter declares, planner obeys
   (ADR-011/012 extension).
2. **Discovery** — `discover(override: Option<Vec<NetPath>>, sysfs: &dyn
   Sysfs)` reads `/sys/class/net` (injected reader for tests): operstate
   `up`, name ≠ `lo`, each → `SourceAddr(addr_of(iface))` when the address
   can be resolved, else `BindDevice(name)`. Discovery failure or an empty
   set degrades to `[NetPath::Default]` — **never a job failure** (the
   issue's degradation rule). The user override list wins verbatim.
3. **Per-path prober** — `probe_path(path, ...)` is `prober::probe_link`
   with the bound client (the existing K=1 phase); a path measuring 0
   throughput is excluded and journaled `path_excluded {iface, why}`.
   Per-path profiles reuse `LinkProfile`'s shape (`single_bps` per path).
4. **Scheduler (pure, unit-tested)** — `plan_weights(profiles) -> Vec<(path,
   weight)>` weights ∝ measured bps; `assign(pieces, weights)` is weighted
   round-robin (largest-remainder so a 3:1 weight splits deterministically);
   per-path AIMD `step(weight, signal)` mirrors `fleet::AdaptiveFleet` (AI
   on clean piece, MD on error spike) — the #43 realign rules stay
   per-session, the path weight is the outer loop.
5. **Journal** — `path_plan {paths:[{mode, target, bps}],
   assignment_hint}` at the probe boundary; exclusions as
   `path_excluded`; per-session drift already absorbed by #43.
6. **Scope guard** — client-per-path via the adapters' existing session
   map seams (tus #50 lane, S3); one-shot HTTP stays single-path
   (`Default`). Concatenation composes: N partials per path × P paths.
7. **JobSpec** — additive optional `paths: Vec<String>` (serde default,
   empty = auto-discover; the user-overridable list). Identity contract
   otherwise untouched.

## RED first

`ium-core/tests/netpath_plan.rs`: capability truth table on this OS,
discovery filter (up/down/lo/override/fallback), weighted round-robin
(largest-remainder, deterministic), exclusion of a 0-bps path, per-path AIMD
step bounds, tus composition N×P. RED run = compile failure (module absent)
on a clean tree. Then the module, then GREEN.

## Live drill (the M8 acceptance)

Two source-bound paths through the throttle proxy pair (`per-flow` at C per
path): aggregate ≥ 2×S1 measured through the MANAGED lane; single-path leg
stays ≈ S1. CI runs the scheduler suite + a capability-gated smoke; the
two-real-uplink field measurement stays the owner's drill (documented, not
faked) — same honesty contract as #55's legs.

## GREEN evidence (2026-11-05, `tus-test.sh --multipath`)

The discriminating shape is the SHARED-BEARER proxy (`--total-rate`): one
bearer caps the aggregate no matter how many flows ride it, so the legs hold
bytes and session count constant and vary ONLY the bearer count.

| Leg | Bearers | Partials | Aggregate | Meaning |
|-----|---------|----------|-----------|---------|
| single-bearer | 1 (`--total-rate 64 KiB/s`) | 2 (one bond) | **57.7 KiB/s** | one bearer holds the aggregate at C — bonding does NOT multiply a bearer it does not own (the honesty baseline; the #49 waterline parked the fleet exactly as designed) |
| dual-bearer | 2 × 64 KiB/s | 2 (one per bond) | **98.7 KiB/s** | the two partials saturated their own bearers CONCURRENTLY (FLOW-WIRE ~62.6 KiB/s each) — the lane sums what the paths own |

**BONDED SUM = 1.71× ≥ the 1.5× classification bar.** The 1.71 vs ideal
2.0 gap is the drill's own wall-clock arithmetic (probe phase, session
creation, finalize/verify ride inside the aggregate denominator) — recorded,
not hidden. Byte-exact read-back on both legs; `path_plan` journaled with 2
live paths on the bonded leg; NO `path_plan` on the single leg (no
`IUM_PATHS` → degradation = pre-#51 byte-for-byte). kill-9 drill re-proven
vs live tusd in the same run. Suite: 199/199 (181 + 17 netpath + the drill).
