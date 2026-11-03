#!/usr/bin/env bash
# stress_tus_flakes.sh (issue #37) — bury the #63/#64 flake faces.
#
# The two CI flakes were timing races in the adapter↔server wire dance:
#   #63  tus_bogus_session_maps_to_session_lost   (windows, vs tusd)
#   #64  tus_offset_conflict_realigns_from_server_offset (ubuntu, vs @tus/server)
# This harness boots EACH pinned server implementation and hammers the
# whole adapter_tus suite N times against it. A surviving flake shows up as
# a non-zero failure count; all-zeros is the evidence the faces are dead.
#
# Usage: bash scripts/stress_tus_flakes.sh [ITERATIONS]
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
# The persistent toolchain lives in .toolchain/ — never assume the ambient PATH.
. "$ROOT/scripts/rust-env.sh"
ITER="${1:-10}"
FAILED_TOTAL=0

say() { echo "[stress-tus] $*"; }

run_suite() { # $1 label, $2 endpoint, $3 server tag
  local label="$1" endpoint="$2" tag="$3" fails=0
  for i in $(seq 1 "$ITER"); do
    if ! IUM_TUS_ENDPOINT="$endpoint" IUM_TUS_SERVER="$tag" \
        cargo test -q -p ium-core --test adapter_tus > /tmp/stress-tus-last.log 2>&1; then
      fails=$((fails + 1))
      cp /tmp/stress-tus-last.log "/tmp/stress-tus-fail-${label}-${i}.log"
      say "  [$label] iter $i FAILED (log kept)"
    fi
  done
  say "$label: $fails/$ITER iterations failed"
  FAILED_TOTAL=$((FAILED_TOTAL + fails))
}

# --- lane 1: the loopback recorder (the shell e2e's own TUS lane) ----------
mkdir -p /tmp/stress-rec
"$ROOT/target/debug/loopback_recorder" --port 22901 \
  --record-file /tmp/stress-rec/rec.json > /tmp/stress-rec/rec.log 2>&1 &
REC_PID=$!
sleep 0.5
run_suite recorder "http://127.0.0.1:22901/files" loopback-recorder
kill $REC_PID 2>/dev/null

# --- lane 2: @tus/server 2.4.7 (the early-reject wire fact, #33/#37) -------
NODE_DIR=/tmp/stress-tusnode
rm -rf "$NODE_DIR" && mkdir -p "$NODE_DIR"
(cd "$HERE/tus-node-lane" && npm ci --no-audit --no-fund --loglevel=error > /dev/null 2>&1)
node "$HERE/tus-node-lane/server.mjs" "$NODE_DIR" 22902 > /tmp/stress-tusnode.log 2>&1 &
NODE_PID=$!
for _ in $(seq 1 20); do curl -s -o /dev/null "http://127.0.0.1:22902/" && break; sleep 0.25; done
if curl -s -o /dev/null "http://127.0.0.1:22902/"; then
  run_suite tus-node "http://127.0.0.1:22902/files/" tus-node
else
  say "tus-node lane failed to boot:"; tail -5 /tmp/stress-tusnode.log
  FAILED_TOTAL=$((FAILED_TOTAL + 1))
fi
kill $NODE_PID 2>/dev/null

# --- lane 3: tusd 2.10.1 (the reference; CI's canonical lane) --------------
TUSD_BIN=/tmp/stress-tusd/tusd
if [ ! -x "$TUSD_BIN" ]; then
  mkdir -p /tmp/stress-tusd
  VER=2.10.1
  case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) TUSD_ASSET="tusd_linux_amd64.tar.gz" ;;
    Darwin/arm64) TUSD_ASSET="tusd_darwin_arm64.tar.gz" ;;
    Darwin/x86_64) TUSD_ASSET="tusd_darwin_amd64.tar.gz" ;;
    *) TUSD_ASSET="" ;;
  esac
  if [ -n "$TUSD_ASSET" ]; then
    curl -fsSL -o /tmp/stress-tusd/tusd.tgz \
      "https://github.com/tus/tusd/releases/download/v${VER}/${TUSD_ASSET}" \
      && tar -xzf /tmp/stress-tusd/tusd.tgz -C /tmp/stress-tusd \
      && mv /tmp/stress-tusd/tusd_*_amd64/tusd "$TUSD_BIN" 2>/dev/null \
      || mv /tmp/stress-tusd/tusd_*_arm64/tusd "$TUSD_BIN" 2>/dev/null \
      || true
    chmod +x "$TUSD_BIN" 2>/dev/null || true
  fi
fi
if [ -x "$TUSD_BIN" ]; then
  mkdir -p /tmp/stress-tusd/data
  "$TUSD_BIN" -host 127.0.0.1 -port 22903 -upload-dir /tmp/stress-tusd/data > /tmp/stress-tusd.log 2>&1 &
  TUSD_PID=$!
  for _ in $(seq 1 20); do curl -s -o /dev/null "http://127.0.0.1:22903/" && break; sleep 0.25; done
  run_suite tusd "http://127.0.0.1:22903/files/" tusd
  kill $TUSD_PID 2>/dev/null
else
  say "tusd binary unavailable (download failed?) — lane skipped"
fi

say "TOTAL failed iterations: $FAILED_TOTAL"
exit $(( FAILED_TOTAL > 0 ? 1 : 0 ))
