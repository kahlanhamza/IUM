#!/usr/bin/env bash
# app-e2e.sh (issue #35) — drive the REAL shell once: build everything, boot
# the loopback recorder(s), start tauri-driver under xvfb, run the env-gated
# WebDriver suite. CI is the proof lane (WebKitGTK + xvfb live there); the
# script fails loudly when a prerequisite is missing instead of faking green.
#
# Usage: bash scripts/app-e2e.sh [--skip-build]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
WORK="$(mktemp -d /tmp/ium-e2e.XXXXXX)"
trap 'kill $(jobs -p) 2>/dev/null || true' EXIT

say() { echo "[app-e2e] $*"; }
die() { echo "[app-e2e] FATAL: $*" >&2; exit 1; }

SKIP_BUILD=0
[ "${1:-}" = "--skip-build" ] && SKIP_BUILD=1

# --- 0. prerequisites ------------------------------------------------------
command -v xvfb-run >/dev/null 2>&1 || die "xvfb-run not found (apt install xvfb)"
command -v WebKitWebDriver >/dev/null 2>&1 || die "WebKitWebDriver not found (apt install webkit2gtk-driver)"
command -v tauri-driver >/dev/null 2>&1 || die "tauri-driver not found (cargo install tauri-driver --locked)"
command -v node >/dev/null 2>&1 || die "node not found"

if [ "$SKIP_BUILD" -eq 0 ]; then
  # --- 1. frontend + shell build ------------------------------------------
  say "building frontend (npm ci + tsc strict + vite build)"
  (cd "$ROOT/ium-app" && { [ -d node_modules ] || npm ci --no-audit --no-fund --loglevel=error; } && npm run build)
  say "building shell + recorder (cargo)"
  (cd "$ROOT" && cargo build -p ium-app --bin ium-app --features ium-app/custom-protocol)
  (cd "$ROOT" && cargo build -p ium-core --bin loopback_recorder)
fi

APP_BIN="$ROOT/target/debug/ium-app"
REC_BIN="$ROOT/target/debug/loopback_recorder"
[ -x "$APP_BIN" ] || die "app binary missing at $APP_BIN (build first)"
[ -x "$REC_BIN" ] || die "recorder binary missing at $REC_BIN (build first)"

# --- 2. source files: real patterned payloads ------------------------------
python3 - "$WORK" <<'PY'
import sys
work = sys.argv[1]
def pattern(path, mib):
    size = mib * 1024 * 1024
    with open(path, "wb") as f:
        off = 0
        chunk = bytearray(range(251)) * (64 * 1024 // 251 + 1)
        while off < size:
            n = min(64 * 1024, size - off)
            f.write(chunk[:n]); off += n
pattern(f"{work}/e2e-a.bin", 24)
pattern(f"{work}/e2e-b.bin", 16)
print(f"{work}/e2e-a.bin {work}/e2e-b.bin")
PY
FILE_A="$WORK/e2e-a.bin"
FILE_B="$WORK/e2e-b.bin"

# --- 3. the loopback recorders (throttled: the e2e controls the clock) -----
REC_A="$WORK/recorder-a.json"
REC_B="$WORK/recorder-b.json"
PORT_A=$(( (RANDOM % 2000) + 23000 ))
PORT_B=$(( (RANDOM % 2000) + 25000 ))
"$REC_BIN" --port "$PORT_A" --throttle-bps 2097152 --record-file "$REC_A" > "$WORK/rec-a.log" 2>&1 &
"$REC_BIN" --port "$PORT_B" --throttle-bps 1048576 --record-file "$REC_B" > "$WORK/rec-b.log" 2>&1 &
sleep 1
grep -q listening "$WORK/rec-a.log" || die "recorder A failed to boot: $(cat "$WORK/rec-a.log")"
grep -q listening "$WORK/rec-b.log" || die "recorder B failed to boot: $(cat "$WORK/rec-b.log")"

# --- 4. tauri-driver under xvfb (WebKitWebDriver spawns the app) -----------
# WebKitGTK's DMABUF renderer is the known crash/wedge source on virtual
# displays (no GPU under xvfb) — standard Tauri CI practice: disable it.
export WEBKIT_DISABLE_DMABUF_RENDERER=1
say "starting tauri-driver (WebDriver 4444, native 4445) under xvfb"
xvfb-run -a --server-args="-screen 0 1440x900x24" \
  tauri-driver --port 4444 --native-port 4445 > "$WORK/driver.log" 2>&1 &
DRIVER_PID=$!
for i in $(seq 1 40); do
  curl -s -o /dev/null "http://127.0.0.1:4444/status" && break
  sleep 0.5
done
curl -s "http://127.0.0.1:4444/status" | grep -q ready || die "tauri-driver not ready: $(tail -25 "$WORK/driver.log")"

# --- 5. the env-gated WebDriver suite --------------------------------------
say "running ium-e2e (driver 4444, recorders $PORT_A/$PORT_B)"
export IUM_E2E_DRIVER="http://127.0.0.1:4444"
export IUM_E2E_APP="$APP_BIN"
export IUM_E2E_FILE_A="$FILE_A"
export IUM_E2E_FILE_B="$FILE_B"
export IUM_E2E_ENDPOINT_A="http://127.0.0.1:$PORT_A/upload"
export IUM_E2E_ENDPOINT_B="http://127.0.0.1:$PORT_B/upload"
export IUM_E2E_RECORD_A="$REC_A"
export IUM_E2E_RECORD_B="$REC_B"
set +e
(cd "$ROOT" && cargo test -p ium-e2e --test e2e_shell -- --nocapture)
RC=$?
set -e

say "--- recorder A: $(cat "$REC_A" 2>/dev/null || echo 'no record')  B: $(cat "$REC_B" 2>/dev/null || echo 'no record')"
if [ $RC -ne 0 ]; then
  say "--- app/driver panic forensics:"
  grep -n -B2 -A12 "panicked at" "$WORK/driver.log" | head -60 || true
  say "--- driver log tail:"
  tail -25 "$WORK/driver.log" || true
  die "e2e FAILED (rc=$RC) — workdir $WORK kept for forensics"
fi
say "GREEN: the shell was driven — create/progress/pause/resume/done/inspect + cancel-no-object"
