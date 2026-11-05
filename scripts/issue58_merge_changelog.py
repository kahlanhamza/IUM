#!/usr/bin/env python3
"""#58: merge [Unreleased] + the stale [0.1.0] seed into ONE
`## [0.1.0] — 2026-11-05` release section; write the 16 missing issue
entries (facts sourced from STATUS.md rows only); keep every existing
entry VERBATIM. Deterministic re-parse of the current file; fails loudly
if any expected header or entry is missing."""
import re
import sys

PATH = "/home/z/my-project/CHANGELOG.md"
lines = open(PATH).read().splitlines()


def idx(header):
    hits = [i for i, l in enumerate(lines) if l.strip() == header]
    assert len(hits) == 1, f"header not unique/missing: {header!r} -> {hits}"
    return hits[0]


def blocks(seg):
    """Split a segment into entry blocks starting at ANY top-level bullet."""
    out, cur = [], None
    for l in seg:
        if re.match(r"^- ", l):
            if cur is not None:
                out.append(cur)
            cur = [l]
        elif cur is not None:
            cur.append(l)
    if cur is not None:
        out.append(cur)
    res = []
    for b in out:
        while b and not b[-1].strip():
            b.pop()
        res.append("\n".join(b).rstrip())
    return res


def entry_num(b):
    m = re.match(r"^- #(\d+)", b)
    if m is None:
        sys.exit(f"Unreleased-Added bullet without an issue number: {b[:60]!r}")
    return int(m.group(1))


H_UNREL = idx("## [Unreleased]")
H_ADD = idx("### Added")
H_FIX = idx("### Fixed")
H_M5 = idx("### Added — M5: API freeze, docs & real-data evidence")
H_OLD = idx("## [0.1.0] — 2026-11-02")
H_ENG = idx("### Added — engine (M1, Sprint 0)")
H_TR = idx("### Added — transports & drills (M3)")
H_APP = idx("### Added — app shell (M4)")
H_CHG = idx("### Changed")
H_PROC = idx("### Process")

header = lines[:H_UNREL]
old_intro = [l for l in lines[H_OLD + 1:H_ENG]]
while old_intro and not old_intro[-1].strip():
    old_intro.pop()
engine_b = blocks(lines[H_ENG + 1:H_TR])
trans_b = blocks(lines[H_TR + 1:H_APP])
app_b = blocks(lines[H_APP + 1:H_CHG])
m5_b = blocks(lines[H_M5 + 1:H_OLD])
fix_b = blocks(lines[H_FIX + 1:H_M5])
chg_b = blocks(lines[H_CHG + 1:H_PROC])
proc_b = blocks(lines[H_PROC + 1:])

# Unreleased ### Added entries, classified by milestone ground truth
# (GitHub milestones + STATUS rows; #27/#28/#29 = the 2026-11-03
# stress-evidence tranche between M5 and M6).
unrel_add = blocks(lines[H_ADD + 1:H_FIX])
assert unrel_add, "no entries under Unreleased ### Added"
STRESS = {27, 28, 29}
M6 = {32, 33}
M8 = {48, 49, 50, 55}
M9 = {56, 57, 63}
stress_b, m6_old, m8_old, m9_old = [], [], [], []
for b in unrel_add:
    n = entry_num(b)
    if n in STRESS:
        stress_b.append(b)
    elif n in M6:
        m6_old.append(b)
    elif n in M8:
        m8_old.append(b)
    elif n in M9:
        m9_old.append(b)
    else:
        sys.exit(f"unclassified Unreleased-Added entry: #{n}")

new = {}

new[34] = """- #34 — the journal seam (M6): `set_job_state_event` is ATOMIC — a state and
  its event commit together, the half-transition RED closed; honest Paused
  park on mid-transfer ENOSPC; real-OS EFBIG pinned via RLIMIT_FSIZE; soak
  bounds pinned (16,391 events → 2.4 MB, WAL→0, RSS +5.5 MiB); the longhaul
  leg runs vs SeaweedFS with honest timebox-pause and fresh-journal replay
  byte-exact."""

new[35] = """- #35 — real-shell WebDriver e2e (M6): the `ium-e2e` crate drives the
  PACKAGED shell (WebKitGTK under xvfb) through create → live progress →
  inspector — the UI's truth asserted on the real app, 3-OS CI leg; later
  hardened by #37's tus leg."""

new[41] = """- #41 — the link prober (M7): S1 canary + K independent sessions (canary/K —
  same wall time both phases), classify PerFlowCap (SK ≥ 2×S1) / BearerCap
  (<1.5×) / Mixed; the piece = 20 s of measured wire clamped lane-floor..max;
  drills prove both shapes (per-flow SK/S1 = 4.0× → fleet 4; bearer 1.0× →
  fleet 1). Wired into the managed flow by #48."""

new[42] = """- #42 — the 600 s wall GONE (M7): no total request timeout; the progress
  tripwire is `read_timeout(idle)` (30 s default, `with_idle` compresses for
  drills) — a stalled link fails loudly, a slow-but-moving link runs forever;
  the RED leg measured at 2 KB/s, and the moved()-watchdog idea was proven
  dead en route (kernel/hyper buffers absorb MiB before backpressure)."""

new[43] = """- #43 — the realign dance is a bounded loop (M7): ≤4 rounds, beyond →
  Transient; Unknown was reserved and then REMOVED entirely when #44 showed
  beyond-lease drift is our own resplit; waste reckoning settled once per
  call; the run-#78 CI post-mortem added the ENTRY HEAD as the alignment
  point — exact moved−wasted, the doomed blind PATCH gone."""

new[44] = """- #44 — the lane-aware chunk floor (M7): `ProbeReport.min_chunk_bytes` rides
  ADR-011 — tus declares 64 KiB, the planner (plan/resplit/cap-replan) obeys
  `declared_lane_floor` clamped [1 KiB, CHUNK_MIN], so pieces confirm in ~30 s
  at the owner's bad-moment rate; the drill forced two real fixes:
  resplit-induced beyond-lease drift confirms in full (single-writer lane) and
  423 Locked is a WAIT not a failure (`ErrorKind::Busy` — uncharged,
  un-evicted)."""

new[45] = """- #45 — the throttled CI ladder 3/8/50/160 KB/s (M7): 99%+ of wire at every
  rung through the real engine (3070.5 / 8186 / 51081 / 162884 B/s achieved),
  byte-exact, overhead MEASURED 0.72%→0.05%; wired into `stress.yml`
  (`IUM_THROTTLE_LADDER=1`), the PR lane keeps compressed drills — the tail
  was found in the field because no drill went below the wall; never again."""

new[46] = """- #46 — the research track (M7): `docs/07_Speed_Architecture.md` on verified
  sources (RFC 8684, the QUIC multipath draft, RFC 3449, RFC 6330, tus
  concatenation) with the owner's research brief ingested verbatim; the
  honest bound — nothing beats the sum of the summoned bearers; research
  REACHES the bound and AGGREGATES bearers (landed as #51 and #52)."""

new[51] = """- #51 — the paths lane (M8 rung 4, the chunk pool sums independent bearers):
  `netpath` — NetPath = Default/SourceAddr/BindDevice, reqwest-native
  bindings (`interface()` = SO_BINDTODEVICE/IP_BOUND_IF, cfg-gated, Windows
  loud-skip; source-addr dual-path needs NO privilege); discovery and
  degradation journaled (`path_plan`/`path_excluded` at the probe boundary,
  `IUM_PATHS`), weighted round-robin + per-path AIMD pure-fn tested, per-path
  prober, fleet seed composes live-paths × per-path fleet, bonded concat
  lane; the `tus-test.sh --multipath` drill: single 57.7 vs dual 98.7 KiB/s =
  1.71× ≥ the 1.5× bar, byte-exact both legs; no `IUM_PATHS` = pre-#51
  byte-for-byte."""

new[52] = """- #52 — the FEC lane (M8 rung 5): `fec` pure module (RFC 6330, crate raptorq
  1.8.1, wire facts W1–W13 rehearsed live in `scripts/wirefacts52/`, pinned
  vectors in the contract tests) + `adapter_fec::FecAdapter` — the client
  half of relay mode: the group grid derives from source size ALONE
  (resplit-stable by construction), per-group lane instances, eager sessions
  at probe with the `fec_plan` journaled; any K of (K+R) rebuilds byte-exact
  — proven LIVE end-to-end: R erasures per group on the RECEIVED streams
  after a managed job → source exact; wire 1.2503× at every loss rung
  (0/3/5%) with the honest verdict: parity wins only on an erasure channel."""

new[53] = """- #53 — the entropy-gated zstd lane (M8 rung 6, the Cause-D byte lever):
  `compress` module (entropy gate = head/mid/tail 64 KiB samples at fast
  level 3, bar 1.15, ratio honesty pinned; streaming checksummed zstd encode;
  decode+hash integrity primitive; statvfs disk check) +
  `adapter_compress::CompressAdapter` — single-chunk shape: the ARCHIVE is
  the wrapped lane's source, so kill-9 / zero-re-upload / 409-realign /
  verify are inherited structurally; the manifest (mode/level/both
  hashes/sizes/ratio) rides the probe event; decline paths = plain jobs with
  the reason journaled (`IUM_NO_COMPRESS` kill-switch); live drill: 86.6%
  saved (4 MiB → 562 KB), video-like bytes declined in 0.03 s."""

new[37] = """- #37 — the CI-debt audit (REOPENED by the owner — runs #62–#69 all failed
  while new pushes kept landing): app-e2e leg A ran `http_oneshot`, which
  plans ONE chunk — the honest Σ-confirmed percent cannot move mid-transfer,
  so the progress assert was unsatisfiable BY CONSTRUCTION (runs #65–#69
  failed identically: a design flaw, not a flake). Fix: the loopback recorder
  gains a real TUS lane (12/12 of the adapter's own wire suite passes against
  it), leg A selects tus with 6 connections, the assert polls with a 45 s
  deadline; flake faces verified dead (`stress_tus_flakes.sh` 36/36)."""

new[54] = """- #54 — the pause drill stopped betting on clocks: run #93's Windows-only
  red was a drill-side flake (a pauser slept 5 ms and unwrapped — Windows
  ~15.6 ms ticks plus runner starvation let the job reach Done first, and
  `pause()` honestly refused); fix = the mock's drill wedge (`arm_wedge(0)`,
  the pauser polls `wedge_engaged`) makes mid-flight pause DETERMINISTIC;
  ZERO production code involved (identical code green in #90–#92); rerun
  green, flake proven on the record."""

new[21] = """- #21 — CI forensics + engine re-proof on the owner's question ("why did CI
  fail — does the engine really work? upload must be fast even on slow
  connection"): all 40 runs audited, 4 failures root-caused, the engine
  re-proven on the record."""

new[31] = """- #31 — the boundary register (`docs/08_Boundary_Register.md`): the entity/
  edge map + seam table — what breaks what, pinned by which test; a
  checkable repo artifact, not chat prose."""

new[38] = """- #38 — README to product standard: badges (ci/nightly/stress), highlights,
  honest scope; roadmap truth synced."""

new[1] = """- M2 hardening & honesty (#1–#5): `verify` is a REQUIRED trait method (#1) —
  the seed of ADR-010's verify-as-commit-gate; journal rows-affected checks on
  `replace_chunks` DELETE (#2); MockAdapter call-log observability parity
  (#3); the nightly chunk-pool fuzz + coverage lane (#4); the planless-jobs
  invariant pinned — every committed job has ≥1 chunk (#5)."""

INTRO_ADD = """This cut closes M1→M9 (the frozen facade grew the speed architecture —
measured profiles, the adaptive fleet, bonding, FEC, compression — and new
transport lanes: Feishu Drive and the rclone bridge over 74 backends) and
lands the first M10 rung. 258 core tests green on the tagged tree; the
guarantees are the loop's measured ones: verify-before-declare (ADR-010),
zero-re-upload resume on every lane, the journal as the only truth (ADR-002)."""

out = []
out += header
out += ["## [Unreleased]", ""]
out += ["## [0.1.0] — 2026-11-05", ""]
out += old_intro
out += ["", INTRO_ADD, ""]
for h, bs in [
    ("### Added — engine (M1–M2, Sprint 0)", [new[1]] + engine_b),
    ("### Added — transports & drills (M3)", trans_b),
    ("### Added — app shell (M4)", app_b),
    ("### Added — M5: API freeze, docs & real-data evidence", m5_b),
    ("### Added — the stress-evidence tranche (2026-11-03, between M5 and M6)",
     stress_b),
    ("### Added — M6: production readiness — the probe generation",
     m6_old + [new[34], new[35]]),
    ("### Added — M7: speed architecture — measured profiles → the IDM brain",
     [new[k] for k in (41, 42, 43, 44, 45, 46)]),
    ("### Added — M8: absolute speed — the prober drives, the fleet adapts, "
     "the lanes multiply", m8_old + [new[51], new[52], new[53]]),
    ("### Added — M9–M10: new transports", m9_old),
    ("### Fixed", fix_b + [new[37], new[54]]),
    ("### Changed", chg_b),
    ("### Process", proc_b + [new[21], new[31], new[38]]),
]:
    out += [h, ""]
    for b in bs:
        out += [b, ""]
    if not bs:
        sys.exit(f"empty bucket: {h}")

text = "\n".join(out).rstrip() + "\n"
open(PATH, "w").write(text)

# verification pass
final = text.splitlines()
bullets = [int(re.match(r"^- #(\d+)", l).group(1)) for l in final
           if re.match(r"^- #\d+", l)]
distinct = sorted(set(bullets))
missing = [n for n in range(6, 65)
           if n not in distinct and n not in (58, 59, 60, 61, 62, 64)]
print("bullets:", len(bullets), "distinct:", len(distinct))
print("missing (closed-issue audit, open #59-#62/#64 excluded):", missing)
hdrs = [l for l in final if l.startswith("## ")]
print("sections:", hdrs)
assert not missing, "still-missing entries"
print("OK — merge complete, no missing entries")
