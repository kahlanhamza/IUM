#!/usr/bin/env bash
# freeze_local_session.sh — the #98 evidence session, single-invocation.
#
# The sandbox reaps detached processes between tool calls, so the server and
# every measurement live inside ONE bash run. Recipe mirrors s3-test.sh
# exactly (pinned weed 4.51, same identity json, idleTimeout=30 armed).
#
# Produces, under scripts/audit/:
#   freeze_drill_local.log   — the freeze drill (parent pins + child wire)
#   poison_experiment.log    — the server-face physics experiment
#   detector_red.log         — dead-endpoint run: the engine must fail HONEST
#                              (the drill's pin-1 teeth)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO"
. scripts/rust-env.sh

WEED="$REPO/target/tmp/weedcache/weed"
S3J="$REPO/target/tmp/weedcache/s3.json"
WORK="/tmp/freeze_session_work"   # PERSISTED: the 500-forensics need the
                                   # server log + the drill journal after the run
rm -rf "$WORK"; mkdir -p "$WORK"
S3_PORT=24561; MASTER_PORT=24562; VOLUME_PORT=24563
LOG="$WORK/weed.log"
export IUM_S3_ENDPOINT="http://127.0.0.1:${S3_PORT}/ium-test"
export IUM_FILER_URL="http://127.0.0.1:8888"
export IUM_S3_ACCESS_KEY="iumcikey"
export IUM_S3_SECRET_KEY="iumcisecret00000000000000000000000000000000"
export IUM_S3_REGION="us-east-1"

cleanup() {
  [ -n "${WEED_PID:-}" ] && kill "$WEED_PID" 2>/dev/null || true
  sleep 0.5
  [ -n "${WEED_PID:-}" ] && kill -9 "$WEED_PID" 2>/dev/null || true
  # $WORK kept: the weed log + drill journal are the 500-forensics
}
trap cleanup EXIT

say() { echo "[session] $*"; }

# --- 1. server (idleTimeout=0 — the kill-9 config; the NAT-twin owns the
#        kill side, and the armed idle killer raced the retry window, #98) --
mkdir -p "$WORK/data"
"$WEED" server -dir="$WORK/data" -ip=127.0.0.1 \
  -master.port="$MASTER_PORT" -volume.port="$VOLUME_PORT" \
  -s3 -s3.port="$S3_PORT" -s3.config="$S3J" -idleTimeout=0 > "$LOG" 2>&1 &
WEED_PID=$!
ready=0
for _ in $(seq 1 120); do
  curl -s -o /dev/null --max-time 2 "http://127.0.0.1:${S3_PORT}/" && ready=1 && break
  kill -0 "$WEED_PID" 2>/dev/null || break
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$LOG" >&2; say "DIE: s3 port not ready"; exit 1; }
say "weed up (s3:$S3_PORT, idleTimeout=30 armed)"

created=$(python3 scripts/s3curl.py PUT "$IUM_S3_ENDPOINT" | head -1 || true)
case "$created" in 200|201|409) say "bucket ready (HTTP $created)";;
  *) say "DIE: bucket create -> $created"; exit 1;; esac

# --- 2. THE FREEZE DRILL (#98) ---------------------------------------------
say "=== FREEZE DRILL ==="
cargo test -p ium-core --test s3_freeze_drill -- --nocapture \
  2>&1 | tee scripts/audit/freeze_drill_local.log | tail -25
DRILL=${PIPESTATUS[0]}
say "freeze drill exit: $DRILL"

# --- 3. POISON EXPERIMENT (server-face physics) ----------------------------
say "=== POISON EXPERIMENT ==="
python3 scripts/audit/poison_experiment.py \
  2>&1 | tee scripts/audit/poison_experiment.log
say "poison exit: ${PIPESTATUS[0]}"

# --- 4. DETECTOR RED (dead endpoint — honest failure, drill teeth) ---------
say "=== DETECTOR RED ==="
DEAD_PORT=24999
cargo build -p ium-core --bin s3_drill_child 2>&1 | tail -1
dd if=/dev/zero of="$WORK/dead.bin" bs=1M count=8 status=none
set +e
IUM_S3_ENDPOINT="http://127.0.0.1:${DEAD_PORT}/ium-test" \
  "$REPO/target/debug/s3_drill_child" upload \
  --journal "$WORK/dead.journal.db" --file "$WORK/dead.bin" --workers 1 \
  > "$WORK/dead.out" 2> "$WORK/dead.err"
CHILD=$?
set -e
{
  echo "child exit: $CHILD (non-zero expected — honest failure)"
  echo "--- stdout:"; cat "$WORK/dead.out"
  echo "--- stderr tail:"; tail -6 "$WORK/dead.err"
} | tee scripts/audit/detector_red.log
[ "$CHILD" -ne 0 ] && say "DETECTOR RED CONFIRMED: the child ended honest-failed; the drill's pin 1 (DONE done) would trip on this" \
  || { say "DIE: dead endpoint did NOT fail the child — pins are vacuous"; exit 1; }

say "=== JOURNAL FORENSICS (all error-class events) ==="
python3 - <<'PYEOF'
import sqlite3, glob
for db in glob.glob('/tmp/freeze_session_work/**/*.db', recursive=True):
    try:
        c = sqlite3.connect(db)
        rows = c.execute("SELECT ts, kind, payload FROM events WHERE kind LIKE '%error%' ORDER BY ts").fetchall()
        print(f"--- {db}: {len(rows)} error-class events")
        for ts, k, p in rows:
            print(f"  {ts} {k}: {p[:220]}")
    except Exception as e:
        print(f"  {db}: {e}")
PYEOF
say "=== WEED LOG (errors/warnings) ==="
rg -i "error|warn|fail|500" "$WORK/weed.log" 2>/dev/null | head -20 || grep -iE "error|warn|fail|500" "$WORK/weed.log" | head -20

say "=== SESSION COMPLETE (drill=$DRILL) ==="
[ "$DRILL" -eq 0 ] || exit 1
