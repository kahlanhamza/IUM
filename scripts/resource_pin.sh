#!/usr/bin/env bash
# resource_pin.sh (issue #95, R2) — the SHELL's resource honesty.
#
# The engine's pins (soak_bounds.rs, load_100.rs) prove the core is bounded;
# this pin proves the part users actually SEE burn battery: the booted app
# process tree must idle near-zero CPU and hold bounded memory.
#
# Modes:
#   --self-test  prove the detectors detect — no WebKit, no display (the
#                CI-economics rule: the sampler is proven with synthetic
#                children, runnable everywhere, ~3 s).
#   live         boot the REAL shell under xvfb, settle, then pin a quiet
#                window: tree CPU-time delta and tree RSS against the pins.
#                CI-only (WebKitGTK lives there — the #61 lesson stands).
#
# Bounds are provisional-generous on purpose (catches a polling loop, which
# burns ~60 000 ms of the 60 s window, and runaway memory) and the receipts
# print everything — they get tightened from CI evidence, not vibes.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
say() { echo "[resource-pin] $*"; }
die() { echo "[resource-pin] FAIL: $*" >&2; exit 1; }

PIN_QUIET_SECS="${IUM_PIN_QUIET_SECS:-60}"
PIN_SETTLE_SECS="${IUM_PIN_SETTLE_SECS:-10}"
PIN_CPU_MS="${IUM_PIN_CPU_MS:-5000}"          # tree CPU-time budget for the quiet window
PIN_RSS_KIB="${IUM_PIN_RSS_KIB:-1572864}"     # 1.5 GiB tree-RSS ceiling

# ---------------------------------------------------------------- sampler ---
# Tree sampler: CPU-time (utime+stime) and RSS for the app pid and every
# descendant. Python because /proc/<pid>/stat parsing (comm may contain
# spaces/parens) and the ppid walk are fiddly, and python3 is a repo
# harness dependency already (s3-test.sh, app-e2e.sh).
sample_tree() { # $1 = root pid
  python3 - "$1" <<'PY'
import os, sys

root = int(sys.argv[1])
CLK = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")

procs = {}
for pid in os.listdir("/proc"):
    if not pid.isdigit():
        continue
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            raw = f.read().decode("utf-8", "replace")
    except OSError:
        continue
    # comm may hold spaces/parens — parse fields AFTER the last ')'.
    # Field map (man 5 proc, 0-indexed after ')'): [0]=state, [1]=ppid,
    # [11]=utime, [12]=stime, [21]=rss (PAGES — pre-landing CI caught the
    # first draft reading [23] = rsslim's neighbor: a 1.1-petabyte tree,
    # the receipt that proves why the self-test now guards the RSS path).
    try:
        after = raw[raw.rindex(")") + 2:].split()
        ppid = int(after[1])
        utime, stime = int(after[11]), int(after[12])
        rss_pages = int(after[21])
    except (ValueError, IndexError):
        continue
    procs[int(pid)] = (ppid, utime + stime, rss_pages * PAGE // 1024)

# Descendants of root via the ppid map (BFS). The root itself may have
# exited between the caller's kill -0 and this walk — that is a real
# finding, not noise: report an empty tree and let the caller fail.
tree, stack = [], [root]
while stack:
    p = stack.pop()
    if p in procs:
        tree.append(procs[p])
        stack.extend(q for q, (pp, _, _) in procs.items() if pp == p)

cpu_ms = sum(t for _, t, _ in tree) * 1000 // CLK
rss_kib = sum(r for _, _, r in tree)
print(f"{cpu_ms} {rss_kib} {len(tree)}")
PY
}

# -------------------------------------------------------------- self-test ---
# Prove the sampler separates a CPU-burning child from a sleeping one, and
# that RSS parses. Without this proof the live pin could vacuously pass.
if [ "${1:-}" = "--self-test" ]; then
  say "self-test: synthetic busy vs sleep children, 2 s window"
  python3 - <<'PY'
import os, subprocess, sys, time

CLK = os.sysconf("SC_CLK_TCK")

def cpu_ms(pid):
    with open(f"/proc/{pid}/stat", "rb") as f:
        raw = f.read().decode("utf-8", "replace")
    after = raw[raw.rindex(")") + 2:].split()
    return (int(after[11]) + int(after[12])) * 1000 // CLK

busy = subprocess.Popen([sys.executable, "-c", "while True: pass"])
sleeper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)"])
try:
    time.sleep(0.3)
    b0, s0 = cpu_ms(busy.pid), cpu_ms(sleeper.pid)
    time.sleep(2.0)
    b1, s1 = cpu_ms(busy.pid), cpu_ms(sleeper.pid)
finally:
    busy.kill(); sleeper.kill(); busy.wait(); sleeper.wait()

bdelta, sdelta = b1 - b0, s1 - s0
print(f"[resource-pin] self-test: busy=+{bdelta}ms sleep=+{sdelta}ms over 2s")
assert bdelta > 1500, f"busy child burned only {bdelta}ms — sampler or scheduler broken"
assert sdelta < 400, f"sleeper burned {sdelta}ms — sampler misreads CPU time"

# The RSS path gets its own proof: the first CI run caught the sampler
# reading the WRONG /proc stat field (a 1.1-petabyte tree sailed through
# the CPU-only self-test). A real process's RSS is between 1 MiB and 64 GiB.
with open(f"/proc/self/stat", "rb") as f:
    raw = f.read().decode("utf-8", "replace")
after = raw[raw.rindex(")") + 2:].split()
PAGE = os.sysconf("SC_PAGE_SIZE")
rss_kib = int(after[21]) * PAGE // 1024
print(f"[resource-pin] self-test: self rss={rss_kib}KiB")
assert 1024 <= rss_kib <= 64 * 1024 * 1024, (
    f"self rss {rss_kib}KiB is not a sane process size — the RSS field parse is broken"
)
print("[resource-pin] SELF-TEST GREEN: the sampler separates burn from idle, and RSS parses real")
PY
  exit 0
fi

# ------------------------------------------------------------------- live ---
APP_BIN="${IUM_PIN_APP:-$ROOT/target/debug/ium-app}"
[ -x "$APP_BIN" ] || die "app binary missing at $APP_BIN (build first, or set IUM_PIN_APP)"
command -v xvfb-run >/dev/null 2>&1 || die "xvfb-run not found (the live pin is CI's WebKitGTK lane)"
command -v python3 >/dev/null 2>&1 || die "python3 not found"

say "booting the shell under xvfb (quiet window ${PIN_QUIET_SECS}s, pins: cpu<=${PIN_CPU_MS}ms rss<=${PIN_RSS_KIB}KiB)"
# The DMABUF renderer is the known crash/wedge source on virtual displays —
# standard Tauri CI practice (the app-e2e lesson, issue #35 family).
export WEBKIT_DISABLE_DMABUF_RENDERER=1
xvfb-run -a --server-args="-screen 0 1440x900x24" "$APP_BIN" > /tmp/ium-resource-pin-app.log 2>&1 &
APP_PID=$!
cleanup() { kill "$APP_PID" 2>/dev/null || true; pkill -P "$APP_PID" 2>/dev/null || true; }
trap cleanup EXIT

# Boot + settle: the window must APPEAR and the tree must go quiet before
# the pin window starts (boot itself burns CPU legitimately).
sleep "$PIN_SETTLE_SECS"
kill -0 "$APP_PID" 2>/dev/null || die "shell died during boot: $(tail -20 /tmp/ium-resource-pin-app.log)"

read -r CPU0 RSS0 N0 < <(sample_tree "$APP_PID")
[ "$N0" -gt 0 ] || die "sampler found no process tree under pid $APP_PID"
say "settled: tree=$N0 procs cpu=${CPU0}ms rss=${RSS0}KiB — pinning the quiet window"

sleep "$PIN_QUIET_SECS"

kill -0 "$APP_PID" 2>/dev/null || die "shell died during the quiet window: $(tail -20 /tmp/ium-resource-pin-app.log)"
read -r CPU1 RSS1 N1 < <(sample_tree "$APP_PID")
CPU_DELTA=$((CPU1 - CPU0))

RECEIPT=$(printf '{"quiet_window_s":%s,"tree_procs_before":%s,"tree_procs_after":%s,"cpu_ms_before":%s,"cpu_ms_after":%s,"cpu_delta_ms":%s,"rss_kib_after":%s,"bounds":{"cpu_ms":%s,"rss_kib":%s}}' \
  "$PIN_QUIET_SECS" "$N0" "$N1" "$CPU0" "$CPU1" "$CPU_DELTA" "$RSS1" "$PIN_CPU_MS" "$PIN_RSS_KIB")
echo "RESOURCE-PIN $RECEIPT"

[ "$CPU_DELTA" -le "$PIN_CPU_MS" ] || die "idle CPU burn: ${CPU_DELTA}ms of CPU time in a ${PIN_QUIET_SECS}s quiet window (a polling loop?)"
[ "$RSS1" -le "$PIN_RSS_KIB" ] || die "tree RSS ${RSS1}KiB past the ${PIN_RSS_KIB}KiB ceiling"
say "GREEN: the booted shell idles honest — cpu +${CPU_DELTA}ms / ${PIN_QUIET_SECS}s, tree rss ${RSS1}KiB"
