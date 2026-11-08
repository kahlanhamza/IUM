#!/usr/bin/env bash
# DEBUG ONLY (not committed): isolated S3-hostile scenario probe with live
# observation. Boots the pinned SeaweedFS exactly like s3-test.sh, starts the
# throttle proxy, runs realworld_stress s3hostile on the 10 MB asset, and
# samples the journal frontier + event kinds + proxy log every 3 s.
set -uo pipefail
cd /home/z/my-project
. scripts/rust-env.sh

RUN=/tmp/s3iso
rm -rf "$RUN"; mkdir -p "$RUN/data"
WORK=$RUN
VERSION=4.51
ASSET="linux_amd64.tar.gz"
URL="https://github.com/seaweedfs/seaweedfs/releases/download/${VERSION}/${ASSET}"

# reuse a previously downloaded weed if present
WEED="$(ls -t /tmp/*/weed /tmp/weed 2>/dev/null | head -1 || true)"
if [ -z "$WEED" ] || [ ! -x "$WEED" ]; then
  echo "[dbg] downloading SeaweedFS $VERSION"
  curl -fsL --retry 3 --retry-delay 2 -o "$WORK/$ASSET" "$URL" || { echo "[dbg] download failed (upstream error?)"; exit 1; }
  tar -xzf "$WORK/$ASSET" -C "$WORK" weed
  WEED="$WORK/weed"
fi
chmod +x "$WEED"

BASE=$(( (RANDOM % 20000) + 20000 ))
S3_PORT=$BASE
cat > "$WORK/s3.json" <<'EOF'
{
  "identities": [
    {
      "name": "ium_ci",
      "credentials": [
        {"accessKey": "iumcikey", "secretKey": "iumcisecret00000000000000000000000000000000"}
      ],
      "actions": ["Read", "Write", "List", "Admin", "Tagging"]
    }
  ]
}
EOF
echo "[dbg] starting weed (s3:$S3_PORT)"
"$WEED" server -dir="$WORK/data" -ip=127.0.0.1 \
  -master.port=$((BASE + 1)) -volume.port=$((BASE + 2)) \
  -s3 -s3.port="$S3_PORT" -s3.config="$WORK/s3.json" -idleTimeout=0 \
  > "$WORK/weed.log" 2>&1 &
WEED_PID=$!
for _ in $(seq 1 120); do
  curl -s -o /dev/null --max-time 2 "http://127.0.0.1:${S3_PORT}/" && break
  sleep 0.5
done
python3 scripts/s3curl.py PUT "http://127.0.0.1:${S3_PORT}/iumrw" | head -1

export IUM_S3_ENDPOINT="http://127.0.0.1:${S3_PORT}/iumrw"
export IUM_FILER_URL="http://127.0.0.1:8888"
export IUM_S3_ACCESS_KEY="iumcikey"
export IUM_S3_SECRET_KEY="iumcisecret00000000000000000000000000000000"
export IUM_S3_REGION="us-east-1"

cargo build -q -p ium-core --bin realworld_stress || exit 1

PPORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
python3 scripts/throttle_proxy.py "$PPORT" 127.0.0.1 "$S3_PORT" \
  --victim-part 2 --victim-rate 98304 --victim-seconds 45 > "$WORK/proxy.log" 2>&1 &
PROXY_PID=$!
sleep 0.5

ENDPOINT="http://127.0.0.1:${PPORT}/iumrw/rw-s3/bbb_20mb.mp4"
ASSET_FILE=${1:-/tmp/bbb_20mb.mp4}
"$REPO_TARGET_UNUSED" 2>/dev/null # placeholder no-op
./target/debug/realworld_stress s3hostile --journal "$RUN/j" --file "$ASSET_FILE" --endpoint "$ENDPOINT" \
  > "$WORK/bin.log" 2>&1 &
BIN_PID=$!

T0=$(date +%s)
while kill -0 "$BIN_PID" 2>/dev/null; do
  NOW=$(date +%s); EL=$((NOW - T0))
  if [ -f "$RUN/j/journal.db" ]; then
    F=$(python3 - "$RUN/j/journal.db" <<'EOF'
import sys, sqlite3
try:
    c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
    s = c.execute("SELECT COALESCE(SUM(bytes_confirmed),0) FROM chunks").fetchone()[0]
    ev = dict(c.execute("SELECT kind, COUNT(*) FROM events GROUP BY kind").fetchall())
    print(f"{s} {ev}")
except Exception as e:
    print("journal:", e)
EOF
)
  else
    F="no-journal-yet"
  fi
  echo "[t=${EL}s] frontier+events: $F"
  tail -2 "$WORK/proxy.log" 2>/dev/null | sed 's/^/    proxy| /'
  if [ "$EL" -ge 240 ]; then echo "[dbg] TIMEBOX"; break; fi
  sleep 3
done

kill "$BIN_PID" 2>/dev/null; wait "$BIN_PID" 2>/dev/null
echo "=== bin.log ==="; cat "$WORK/bin.log"
echo "=== proxy.log ==="; cat "$WORK/proxy.log"
kill "$PROXY_PID" "$WEED_PID" 2>/dev/null
echo "[dbg] done"
