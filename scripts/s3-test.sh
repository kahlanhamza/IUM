#!/usr/bin/env bash
# s3-test.sh — the 3-OS S3 test harness (issue #9; server diversity lane #33).
#
# Downloads the PINNED S3 server binary for this runner and boots it with
# throwaway CI creds, creates the bucket, exports IUM_S3_*, runs the
# env-gated S3 suite (and, with --with-drill, the kill-9 drill), then aborts
# leftover multipart uploads and kills the server.
#
# Servers (--server NAME, default seaweedfs):
#   seaweedfs  SeaweedFS 4.51 — one product, every OS (linux/darwin/windows;
#              Garage v2.1.0 publishes Linux-only, the MinIO premise is dead,
#              see docs/07 §6 amendment). Canonical: carries the kill-9 drill.
#   rclone     rclone 1.75.2 `serve s3` (issue #33 — third S3 implementation,
#              built on rclone's gofakes3 fork). Suite-only: the kill-9 drill
#              semantics were built against SeaweedFS and stay canonical there.
#
# Supply-chain guards (recorded harness facts, #33): SeaweedFS ships per-asset
# .md5 sidecars; rclone ships NO per-asset checksums — the guard is the
# per-release SHA256SUMS file, grepped for the asset line.
#
# Usage: scripts/s3-test.sh [--server seaweedfs|rclone] [--with-drill] [--with-real]
# Requires: bash, curl, tar/unzip, sha256sum, python3|python, cargo (already set up).

set -euo pipefail

VERSION="4.51"
BASE_URL="https://github.com/seaweedfs/seaweedfs/releases/download/${VERSION}"
SERVER="seaweedfs"
RCLONE_VERSION="1.75.2"
RCLONE_BASE_URL="https://downloads.rclone.org/v${RCLONE_VERSION}"
BUCKET="ium-test"
WITH_DRILL=0
WITH_REAL=0
WITH_REALWORLD=0
WITH_LONGHAUL=0
LONGHAUL_ARGS=()
REALWORLD_FILES=()
say() { echo "[s3-test] $*"; }
die() { echo "[s3-test] FAIL: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
  case "$1" in
    # --server NAME: which pinned S3 implementation to run against (#33).
    --server)
      [ $# -ge 2 ] || die "--server needs a value (seaweedfs|rclone)"
      case "$2" in
        seaweedfs|rclone) SERVER="$2" ;;
        *) die "unknown server: $2 (seaweedfs|rclone)" ;;
      esac
      shift ;;
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
    # --with-longhaul FILE [RATE_Bps [TIMEBOX_SECS]] — the long-horizon leg
    # (issue #34) INSTEAD of the standard suites: one real asset through the
    # uniform-throttle proxy (default 512 B/s), persistence telemetry,
    # honest timebox (pause, never cancel), fresh-journal replay to Done.
    --with-longhaul)
      WITH_LONGHAUL=1; shift; LONGHAUL_ARGS=("$@"); break ;;
    *) die "unknown flag: $1" ;;
  esac
  shift
done

if [ "$WITH_DRILL" = 1 ] && [ "$SERVER" != "seaweedfs" ]; then
  die "--with-drill is SeaweedFS-canonical only (kill-9 drill semantics built vs SeaweedFS; see #33)"
fi

PY=python3
command -v python3 >/dev/null 2>&1 || PY=python
command -v "$PY" >/dev/null 2>&1 || die "no python for s3curl.py"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- 1. per-OS asset ------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
if [ "$SERVER" = "rclone" ]; then
  # rclone names macOS "osx" (not darwin); windows asset is a zip; linux amd64
  # zip (the .deb/.rpm are for humans, the zip is what other harnesses script).
  case "$os" in
    Linux)  case "$arch" in x86_64) ASSET="rclone-v${RCLONE_VERSION}-linux-amd64.zip";; aarch64|arm64) ASSET="rclone-v${RCLONE_VERSION}-linux-arm64.zip";; *) die "unsupported linux arch: $arch";; esac ;;
    Darwin) case "$arch" in arm64) ASSET="rclone-v${RCLONE_VERSION}-osx-arm64.zip";; x86_64) ASSET="rclone-v${RCLONE_VERSION}-osx-amd64.zip";; *) die "unsupported mac arch: $arch";; esac ;;
    MINGW*|MSYS*|CYGWIN*) ASSET="rclone-v${RCLONE_VERSION}-windows-amd64.zip" ;;
    *) die "unsupported OS: $os" ;;
  esac
  say "pinned server: rclone ${RCLONE_VERSION} serve s3 (${ASSET})"
else
  case "$os" in
    Linux)  case "$arch" in x86_64) ASSET="linux_amd64.tar.gz";; aarch64|arm64) ASSET="linux_arm64.tar.gz";; *) die "unsupported linux arch: $arch";; esac ;;
    Darwin) case "$arch" in arm64) ASSET="darwin_arm64.tar.gz";; x86_64) ASSET="darwin_amd64.tar.gz";; *) die "unsupported mac arch: $arch";; esac ;;
    MINGW*|MSYS*|CYGWIN*) ASSET="windows_amd64.zip" ;;
    *) die "unsupported OS: $os" ;;
  esac
  say "pinned server: SeaweedFS ${VERSION} (${ASSET})"
fi

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

if [ "$SERVER" = "rclone" ]; then
  # -f is the #89 guard: without it an upstream 504 HTML body is saved with exit 0
  # and poisons the sidecar; --retry 3 + --retry-delay 2 rides out CDN brown-outs.
  curl -fsSL --retry 3 --retry-delay 2 -o "$WORK/$ASSET" "$RCLONE_BASE_URL/$ASSET" \
    || die "download $RCLONE_BASE_URL/$ASSET — upstream error (HTTP 4xx/5xx?)"
  curl -fsSL --retry 3 --retry-delay 2 -o "$WORK/SHA256SUMS" "$RCLONE_BASE_URL/SHA256SUMS" \
    || die "download SHA256SUMS — upstream error (HTTP 4xx/5xx?)"
  expected_sha256="$(awk -v a="$ASSET" '$2==a {print $1}' "$WORK/SHA256SUMS" | tr -d '[:space:]')"
  actual_sha256="$(sha256sum "$WORK/$ASSET" | cut -d' ' -f1)"
  echo "$expected_sha256" | grep -Eq '^[0-9a-fA-F]{64}$' \
    || die "SHA256SUMS has no 64-hex digest for $ASSET — upstream served an error page? (supply-chain guard)"
  [ "$actual_sha256" = "$expected_sha256" ] \
    || die "sha256 mismatch for $ASSET (expected ${expected_sha256:-none}, got $actual_sha256) — supply-chain guard"
  say "downloaded + sha256 verified (via SHA256SUMS)"
else
  curl -fsSL --retry 3 --retry-delay 2 -o "$WORK/$ASSET" "$BASE_URL/$ASSET" \
    || die "download $BASE_URL/$ASSET — upstream error (HTTP 4xx/5xx?)"
  curl -fsSL --retry 3 --retry-delay 2 -o "$WORK/$ASSET.md5" "$BASE_URL/$ASSET.md5" \
    || die "download md5 sidecar — upstream error (HTTP 4xx/5xx?)"
  actual_md5="$(md5sum "$WORK/$ASSET" | cut -d' ' -f1)"
  expected_md5="$(tr -d '[:space:]' < "$WORK/$ASSET.md5")"
  echo "$expected_md5" | grep -Eq '^[0-9a-fA-F]{32}$' \
    || die "md5 sidecar for $ASSET is not a 32-hex digest — upstream served an error page? (supply-chain guard)"
  [ "$actual_md5" = "$expected_md5" ] \
    || die "md5 mismatch for $ASSET (expected $expected_md5, got $actual_md5) — supply-chain guard"
  say "downloaded + md5 verified"
fi

case "$ASSET" in
  *.tar.gz) tar -xzf "$WORK/$ASSET" -C "$WORK" weed || tar -xzf "$WORK/$ASSET" -C "$WORK" ;;
  *.zip)    "$PY" -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$WORK/$ASSET" "$WORK" ;;
esac

if [ "$SERVER" = "rclone" ]; then
  # zip layout: rclone-v<ver>-<os>-<arch>/rclone(.exe) — find it portably.
  RCLONE_BIN="$(find "$WORK" -type f \( -name rclone -o -name rclone.exe \) | head -1)"
  [ -n "$RCLONE_BIN" ] || die "rclone binary not found after extract"
  chmod +x "$RCLONE_BIN" 2>/dev/null || true
else
  WEED="$WORK/weed"
  [ -x "$WEED" ] || WEED="$WORK/weed.exe"
  [ -x "$WEED" ] || die "binary not found after extract (no weed / weed.exe)"
  chmod +x "$WEED" 2>/dev/null || true
fi

# --- 2. random free port (rclone) / port triple (SeaweedFS) ----------------
BASE=$(( (RANDOM % 20000) + 20000 ))
S3_PORT=$BASE
if [ "$SERVER" != "rclone" ]; then
  MASTER_PORT=$((BASE + 1)); VOLUME_PORT=$((BASE + 2))
fi

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
if [ "$SERVER" = "rclone" ]; then
  say "starting rclone serve s3 (s3:$S3_PORT)"
  "$RCLONE_BIN" serve s3 --auth-key "iumcikey,iumcisecret00000000000000000000000000000000" \
    --addr "127.0.0.1:${S3_PORT}" "$WORK/data" > "$WORK/rclone.log" 2>&1 &
  WEED_PID=$!   # single kill-handle name reused by cleanup
  LOG="$WORK/rclone.log"
else
  say "starting weed server (s3:$S3_PORT master:$MASTER_PORT volume:$VOLUME_PORT)"
  "$WEED" server -dir="$WORK/data" -ip=127.0.0.1 \
    -master.port="$MASTER_PORT" -volume.port="$VOLUME_PORT" \
    -s3 -s3.port="$S3_PORT" -s3.config="$WORK/s3.json" -idleTimeout=0 \
    > "$WORK/weed.log" 2>&1 &
  WEED_PID=$!
  LOG="$WORK/weed.log"
fi

ready=0
for _ in $(seq 1 120); do
  if curl -s -o /dev/null --max-time 2 "http://127.0.0.1:${S3_PORT}/"; then ready=1; break; fi
  if ! kill -0 "$WEED_PID" 2>/dev/null; then break; fi
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$LOG" >&2; die "S3 port not ready"; }
say "S3 port ready"

# --- 3. bucket -------------------------------------------------------------
created=$("$PY" "$SCRIPT_DIR/s3curl.py" PUT "http://127.0.0.1:${S3_PORT}/${BUCKET}" | head -1 || true)
case "$created" in
  200|201|409) say "bucket ${BUCKET} ready (HTTP $created)" ;;
  *) tail -20 "$LOG" >&2; die "bucket create -> $created" ;;
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
  [ "$SERVER" = "seaweedfs" ] || die "--with-realworld is SeaweedFS-canonical only (#33)"
  [ "${#REALWORLD_FILES[@]}" -ge 1 ] || die "--with-realworld needs at least one file path"
  for f in "${REALWORLD_FILES[@]}"; do
    [ -f "$f" ] || die "real-world asset not found: $f"
  done
  say "running real-world stress drill (issue #22) with ${#REALWORLD_FILES[@]} real asset(s)"
  bash "$SCRIPT_DIR/realworld-stress.sh" "${REALWORLD_FILES[@]}"
  say "GREEN: real-world stress drill passed against SeaweedFS ${VERSION} on ${os}/${arch}"
  exit 0
fi

if [ "$WITH_LONGHAUL" = 1 ]; then
  [ "$SERVER" = "seaweedfs" ] || die "--with-longhaul is SeaweedFS-canonical only (#33/#34)"
  [ "${#LONGHAUL_ARGS[@]}" -ge 1 ] || die "--with-longhaul needs a file path"
  [ -f "${LONGHAUL_ARGS[0]}" ] || die "longhaul asset not found: ${LONGHAUL_ARGS[0]}"
  say "running long-horizon leg (issue #34): ${LONGHAUL_ARGS[*]}"
  bash "$SCRIPT_DIR/longhaul.sh" "${LONGHAUL_ARGS[@]}"
  say "GREEN: longhaul leg passed against SeaweedFS ${VERSION} on ${os}/${arch}"
  exit 0
fi

say "running env-gated S3 suite (adapter_s3)"
cargo test -p ium-core --test adapter_s3

if [ "$WITH_DRILL" = 1 ]; then
  say "running kill-9 drill (s3_drill)"
  cargo test -p ium-core --test s3_drill -- --nocapture
fi

if [ "$WITH_REAL" = 1 ] && [ "$SERVER" = "seaweedfs" ]; then
  say "running real-data drill vs SeaweedFS (real_data, issue #17)"
  cargo test -p ium-core --test real_data -- --nocapture
fi

say "GREEN: S3 tests passed against ${SERVER} on ${os}/${arch}"
