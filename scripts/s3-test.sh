#!/usr/bin/env bash
# s3-test.sh — the 3-OS S3 test harness (issue #9).
#
# Downloads the PINNED SeaweedFS binary for this runner (one product, every
# OS: linux/darwin/windows — Garage v2.1.0 publishes Linux-only binaries, the
# MinIO premise is dead, see docs/07 §6 amendment), boots it on a random free
# port triple with throwaway CI creds, creates the bucket, exports IUM_S3_*,
# runs the env-gated S3 suite (and, with --with-drill, the kill-9 drill),
# then aborts leftover multipart uploads and kills the server.
#
# Usage: scripts/s3-test.sh [--with-drill] [--with-real]
# Requires: bash, curl, tar, md5sum, python3|python, cargo (already set up).

set -euo pipefail

VERSION="4.51"
BASE_URL="https://github.com/seaweedfs/seaweedfs/releases/download/${VERSION}"
BUCKET="ium-test"
WITH_DRILL=0
WITH_REAL=0
WITH_REALWORLD=0
REALWORLD_FILES=()
say() { echo "[s3-test] $*"; }
die() { echo "[s3-test] FAIL: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
  case "$1" in
    --with-drill) WITH_DRILL=1 ;;
    # --with-real: also run the real-data drill (tests/real_data.rs, issue
    # #17) — a REAL ffmpeg-generated video through the engine; skipped loud
    # where ffmpeg is absent.
    --with-real) WITH_REAL=1 ;;
    # --with-realworld FILE [FILE...] — run the real-world stress drill
    # (issue #22) INSTEAD of the standard suites: real film(s) from disk
    # through crash-resume, hostile network, pause/cancel, concurrent fleet.
    --with-realworld)
      WITH_REALWORLD=1; shift; REALWORLD_FILES=("$@"); break ;;
    *) die "unknown flag: $1" ;;
  esac
  shift
done

PY=python3
command -v python3 >/dev/null 2>&1 || PY=python
command -v "$PY" >/dev/null 2>&1 || die "no python for s3curl.py"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- 1. per-OS asset ------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  case "$arch" in x86_64) ASSET="linux_amd64.tar.gz";; aarch64|arm64) ASSET="linux_arm64.tar.gz";; *) die "unsupported linux arch: $arch";; esac ;;
  Darwin) case "$arch" in arm64) ASSET="darwin_arm64.tar.gz";; x86_64) ASSET="darwin_amd64.tar.gz";; *) die "unsupported mac arch: $arch";; esac ;;
  MINGW*|MSYS*|CYGWIN*) ASSET="windows_amd64.zip" ;;
  *) die "unsupported OS: $os" ;;
esac
say "pinned server: SeaweedFS ${VERSION} (${ASSET})"

WORK="$(mktemp -d)"
cleanup() {
  say "aborting leftover multipart uploads on ${BUCKET}"
  "$PY" "$SCRIPT_DIR/s3curl.py" --abort-multiparts \
    "http://127.0.0.1:${S3_PORT}/${BUCKET}" >/dev/null 2>&1 || true
  if [ -n "${WEED_PID:-}" ]; then
    kill "$WEED_PID" >/dev/null 2>&1 || true
    # Windows/git-bash: weed.exe releases file handles asynchronously — wait
    # for the process to actually die before rm, then force it.
    for _ in $(seq 1 20); do
      kill -0 "$WEED_PID" 2>/dev/null || break
      sleep 0.5
    done
    kill -9 "$WEED_PID" >/dev/null 2>&1 || true
  fi
  # Best effort: a lingering handle must never turn a green suite red
  # (runner temp dirs are ephemeral anyway).
  rm -rf "$WORK" 2>/dev/null || true
}
trap cleanup EXIT

curl -sSL --retry 3 -o "$WORK/$ASSET" "$BASE_URL/$ASSET" \
  || die "download $BASE_URL/$ASSET"
curl -sSL --retry 3 -o "$WORK/$ASSET.md5" "$BASE_URL/$ASSET.md5" \
  || die "download md5 sidecar"
actual_md5="$(md5sum "$WORK/$ASSET" | cut -d' ' -f1)"
expected_md5="$(tr -d '[:space:]' < "$WORK/$ASSET.md5")"
[ -n "$expected_md5" ] && [ "$actual_md5" = "$expected_md5" ] \
  || die "md5 mismatch for $ASSET (expected $expected_md5, got $actual_md5) — supply-chain guard"
say "downloaded + md5 verified"

case "$ASSET" in
  *.tar.gz) tar -xzf "$WORK/$ASSET" -C "$WORK" weed || tar -xzf "$WORK/$ASSET" -C "$WORK" ;;
  *.zip)    "$PY" -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$WORK/$ASSET" "$WORK" ;;
esac
WEED="$WORK/weed"
[ -x "$WEED" ] || WEED="$WORK/weed.exe"
[ -x "$WEED" ] || die "binary not found after extract (no weed / weed.exe)"
chmod +x "$WEED" 2>/dev/null || true

# --- 2. random free port triple (no stale-server collisions, any OS) ------
BASE=$(( (RANDOM % 20000) + 20000 ))
S3_PORT=$BASE; MASTER_PORT=$((BASE + 1)); VOLUME_PORT=$((BASE + 2))

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

mkdir -p "$WORK/data"
say "starting weed server (s3:$S3_PORT master:$MASTER_PORT volume:$VOLUME_PORT)"
"$WEED" server -dir="$WORK/data" -ip=127.0.0.1 \
  -master.port="$MASTER_PORT" -volume.port="$VOLUME_PORT" \
  -s3 -s3.port="$S3_PORT" -s3.config="$WORK/s3.json" -idleTimeout=0 \
  > "$WORK/weed.log" 2>&1 &
WEED_PID=$!

ready=0
for _ in $(seq 1 120); do
  if curl -s -o /dev/null --max-time 2 "http://127.0.0.1:${S3_PORT}/"; then ready=1; break; fi
  if ! kill -0 "$WEED_PID" 2>/dev/null; then break; fi
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$WORK/weed.log" >&2; die "S3 port not ready"; }
say "S3 port ready"

# --- 3. bucket -------------------------------------------------------------
created=$("$PY" "$SCRIPT_DIR/s3curl.py" PUT "http://127.0.0.1:${S3_PORT}/${BUCKET}" | head -1 || true)
case "$created" in
  200|409) say "bucket ${BUCKET} ready (HTTP $created)" ;;
  *) tail -20 "$WORK/weed.log" >&2; die "bucket create -> $created" ;;
esac

# --- 4. env + tests --------------------------------------------------------
# CI runners have cargo on PATH already; local runs pick up the persistent
# project toolchain (scripts/rust-env.sh) when it exists.
command -v cargo >/dev/null 2>&1 \
  || { [ -f "$REPO/scripts/rust-env.sh" ] && . "$REPO/scripts/rust-env.sh"; }
command -v cargo >/dev/null 2>&1 || die "cargo not on PATH"

export IUM_S3_ENDPOINT="http://127.0.0.1:${S3_PORT}/${BUCKET}"
export IUM_FILER_URL="http://127.0.0.1:8888"
export IUM_S3_ACCESS_KEY="iumcikey"
export IUM_S3_SECRET_KEY="iumcisecret00000000000000000000000000000000"
export IUM_S3_REGION="us-east-1"
cd "$REPO"

if [ "$WITH_REALWORLD" = 1 ]; then
  [ "${#REALWORLD_FILES[@]}" -ge 1 ] || die "--with-realworld needs at least one file path"
  for f in "${REALWORLD_FILES[@]}"; do
    [ -f "$f" ] || die "real-world asset not found: $f"
  done
  say "running real-world stress drill (issue #22) with ${#REALWORLD_FILES[@]} real asset(s)"
  bash "$SCRIPT_DIR/realworld-stress.sh" "${REALWORLD_FILES[@]}"
  say "GREEN: real-world stress drill passed against SeaweedFS ${VERSION} on ${os}/${arch}"
  exit 0
fi

say "running env-gated S3 suite (adapter_s3)"
cargo test -p ium-core --test adapter_s3

if [ "$WITH_DRILL" = 1 ]; then
  say "running kill-9 drill (s3_drill)"
  cargo test -p ium-core --test s3_drill -- --nocapture
fi

if [ "$WITH_REAL" = 1 ]; then
  say "running real-data drill vs SeaweedFS (real_data, issue #17)"
  cargo test -p ium-core --test real_data -- --nocapture
fi

say "GREEN: S3 tests passed against SeaweedFS ${VERSION} on ${os}/${arch}"
