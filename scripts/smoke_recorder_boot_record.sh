#!/usr/bin/env bash
# Smoke-test the run-#74 fix: the loopback recorder's record file must exist
# from BOOT (all zeros = "served nothing yet"), HEAD must 404 pre-PUT, and a
# PUT aborted mid-body (the cancel leg) must leave the record readable with
# received_bytes == 0.
set -uo pipefail
cd /home/z/my-project
. scripts/rust-env.sh
export CARGO_INCREMENTAL=0

cargo build -p ium-core --bin loopback_recorder 2>&1 | tail -1

W=$(mktemp -d /tmp/rec-smoke.XXXXXX)
PORT=$(( (RANDOM % 2000) + 23000 ))
./target/debug/loopback_recorder --port "$PORT" --throttle-bps 1048576 --record-file "$W/rec.json" > "$W/log" 2>&1 &
REC_PID=$!
trap 'kill $REC_PID 2>/dev/null || true' EXIT
sleep 1

fail() { echo "SMOKE FAIL: $*"; exit 1; }

echo "--- 1. record file at boot (must exist, all zeros):"
[ -f "$W/rec.json" ] || fail "record file missing at boot"
cat "$W/rec.json"
python3 - "$W/rec.json" <<'PY' || fail "boot record not all-zero"
import json, sys
v = json.load(open(sys.argv[1]))
assert v == {"put_count": 0, "received_bytes": 0, "tus_sessions": 0, "tus_bytes": 0, "tus_last": None}, v
PY
echo "boot record all-zero: OK"

echo "--- 2. HEAD before any PUT (the cancel leg's server truth — must be 404):"
code=$(curl -s -o /dev/null -w "%{http_code}" -I "http://127.0.0.1:$PORT/upload")
[ "$code" = "404" ] || fail "HEAD expected 404, got $code"
echo "HEAD 404: OK"

echo "--- 3. PUT aborted mid-body (the #74 cancel scenario):"
# 16 MiB content-length at 1 MiB/s throttle; send only 1 MiB then RST.
python3 - "$PORT" <<'PY' || fail "abort drill errored"
import socket, sys
port = int(sys.argv[1])
s = socket.create_connection(("127.0.0.1", port))
s.sendall(b"PUT /upload HTTP/1.1\r\nHost: x\r\nContent-Length: 16777216\r\n\r\n")
s.sendall(b"x" * (1024 * 1024))
s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, b"\x01\x00\x00\x00\x00\x00\x00\x00")
s.close()  # RST: connection dies mid-body, like a canceled upload
PY
sleep 0.5
echo "--- record after aborted PUT (must still exist, received_bytes == 0):"
cat "$W/rec.json"
python3 - "$W/rec.json" <<'PY' || fail "record after abort wrong"
import json, sys
v = json.load(open(sys.argv[1]))
assert v["received_bytes"] == 0 and v["put_count"] == 0, v
PY
echo "abort leaves zeros: OK"

echo "--- 4. a COMPLETED PUT updates the record (the surface still moves):"
head -c 262144 /dev/zero | curl -s -o /dev/null -w "PUT %{http_code}\n" -X PUT --data-binary @- "http://127.0.0.1:$PORT/upload"
sleep 0.3
cat "$W/rec.json"
python3 - "$W/rec.json" <<'PY' || fail "record after completed PUT wrong"
import json, sys
v = json.load(open(sys.argv[1]))
assert v["put_count"] == 1 and v["received_bytes"] == 262144, v
PY

echo "SMOKE GREEN: record-at-boot + HEAD 404 pre-PUT + abort-zeros + PUT updates"
