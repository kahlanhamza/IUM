#!/usr/bin/env bash
# dbg-tus-drill.sh — hand-driven tus_drill (issue #11 drill) with the journal
# CAPTURED instead of deleted, for the #30 investigation: the nightly caught
# "resume moved <missing>+192 KiB — beyond the 64 KiB rejected-attempt slack".
# This reproduces the drill outside the parent test so the journal's event log
# (reattach / confirms / resplit / stale_* / retries) can be dumped and the
# flush (WIRE - missing) measured per run.
#
# Env: RUNS=N (default 1), HOLD_MS (default 8000), KEEP_TUSD=1 to skip cleanup.
# Prints one summary line per run:  RUN i offset=<n> wire=<n> missing=<n> flush=<n>
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
[ -f scripts/rust-env.sh ] && . scripts/rust-env.sh
command -v cargo >/dev/null 2>&1 || { echo "no cargo" >&2; exit 1; }

TUSD_BIN="/tmp/tusd-reuse/tusd_linux_amd64/tusd"
[ -x "$TUSD_BIN" ] || { echo "tusd missing at $TUSD_BIN (run the pinned download first)" >&2; exit 1; }

RUNS="${RUNS:-1}"
HOLD_MS="${HOLD_MS:-8000}"
SIZE=$((40 * 1024 * 1024))
CHUNK=$((SIZE / 2))
WORK="$(mktemp -d)"
TUS_PORT=$(( (RANDOM % 20000) + 20000 ))
TUS_DATA="$(mktemp -d)"

cleanup() {
  [ -n "${TUSD_PID:-}" ] && kill "$TUSD_PID" >/dev/null 2>&1 || true
  sleep 0.3
  kill -9 "$TUSD_PID" >/dev/null 2>&1 || true
  [ "${KEEP_TUSD:-0}" = "1" ] || rm -rf "$WORK" "$TUS_DATA"
}
trap cleanup EXIT

"$TUSD_BIN" -host 127.0.0.1 -port "$TUS_PORT" -upload-dir "$TUS_DATA" > "$WORK/tusd.log" 2>&1 &
TUSD_PID=$!
for _ in $(seq 1 120); do
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 -X OPTIONS "http://127.0.0.1:${TUS_PORT}/files/" || true)"
  [ "$code" = "200" ] && break
  sleep 0.5
done

export IUM_TUS_ENDPOINT="http://127.0.0.1:${TUS_PORT}/files/"

CHILD="$ROOT/target/debug/tus_drill_child"
[ -x "$CHILD" ] || cargo build -p ium-core --bin tus_drill_child

for i in $(seq 1 "$RUNS"); do
  SRC="$WORK/src40_$i.bin"
  dd if=/dev/urandom of="$SRC" bs=1M count=40 status=none
  JOURNAL="$WORK/drill_$i.journal.db"

  # child 1: upload, frozen after the server ack (parent kills inside the hold)
  "$CHILD" upload --journal "$JOURNAL" --file "$SRC" --hold-ms "$HOLD_MS" \
    > "$WORK/up_$i.out" 2> "$WORK/up_$i.err" &
  UP_PID=$!
  ACK=""
  for _ in $(seq 1 600); do
    ACK="$(sed -n 's/^SERVER_ACK //p' "$WORK/up_$i.out" | head -1)"
    [ -n "$ACK" ] && break
    sleep 0.05
  done
  [ -n "$ACK" ] || { echo "RUN $i NO_SERVER_ACK"; kill -9 "$UP_PID" 2>/dev/null; continue; }
  sleep 0.5
  kill -9 "$UP_PID" 2>/dev/null || true
  wait "$UP_PID" 2>/dev/null || true

  # journal truth at kill: zero confirmed (the window proof)
  python3 - "$JOURNAL" <<'PY' > "$WORK/j1_$i.txt"
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
jobs = db.execute("select id, state from jobs").fetchall()
chunks = db.execute("select idx, state, bytes_confirmed from chunks").fetchall()
ev = db.execute("select count(*) from events").fetchone()[0]
print("jobs", jobs)
print("chunks", chunks)
print("events", ev)
PY

  # child 2: fresh process, fresh adapter — the resume under test
  "$CHILD" resume --journal "$JOURNAL" > "$WORK/res_$i.out" 2> "$WORK/res_$i.err"
  RES=$?

  OFF="$(sed -n 's/^SERVER_OFFSET //p' "$WORK/res_$i.out" | head -1)"
  WIRE="$(sed -n 's/^WIRE //p' "$WORK/res_$i.out" | head -1)"
  DONE="$(sed -n 's/^DONE //p' "$WORK/res_$i.out" | head -1)"
  MISSING=$((SIZE - OFF))
  FLUSH=$((WIRE - MISSING))

  # full event log of the resume (the manager's every journaled move)
  python3 - "$JOURNAL" "$i" <<'PY' > "$WORK/j2_$i.txt"
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
for ts, kind, payload in db.execute("select ts, kind, payload from events order by rowid"):
    p = (payload or "")[:160].replace("\n", " ")
    print(f"EV {kind:28s} {p}")
PY

  echo "RUN $i rc=$RES offset=$OFF wire=$WIRE missing=$MISSING flush=$FLUSH done=$DONE"
  echo "  journal@kill: $(tr '\n' ' ' < "$WORK/j1_$i.txt")"
  if [ "${VERBOSE:-0}" = "1" ]; then
    sed 's/^/  /' "$WORK/j2_$i.txt"
  fi
  cp "$JOURNAL" "$WORK/final_$i.journal.db" 2>/dev/null || true
done

echo "workdir kept: $WORK"
