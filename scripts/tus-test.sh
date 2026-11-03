#!/usr/bin/env bash
# tus-test.sh — the 3-OS TUS test harness (issue #10; second lane #33).
#
# Servers (--server NAME, default tusd):
#   tusd      tus/tusd v2.10.1 — the reference TUS 1.0.0 server, single binary
#             per OS from GitHub Releases with sha256 sidecars. Canonical: it
#             carries the kill-9 drill and the real-data drill (its
#             GET-download extension is how those tests read back bytes).
#   tus-node  @tus/server 2.4.7 + @tus/file-store 2.1.2 (issue #33 — the
#             second independent TUS implementation, npm-pinned via committed
#             package-lock.json — the registry integrity hashes ARE the
#             supply-chain guard). tus 1.0.0 core + creation/termination/
#             expiration, NO GET-download extension: the read-back tests
#             skip LOUDLY on this lane (the delta is recorded on #33).
#
# Boots the pinned server on a random free port with a temp upload dir,
# exports IUM_TUS_ENDPOINT (+ IUM_TUS_SERVER so tests know which wire they
# are on), runs the env-gated TUS suite, then kills the server and removes
# the temp dir (TUS has no multipart-orphan equivalent — the upload dir IS
# the ephemeral state).
#
# Usage: scripts/tus-test.sh [--server tusd|tus-node] [--with-drill] [--with-real]
# Requires: bash, curl, tar/unzip, python3|python, cargo; tus-node lane needs node+npm.

set -euo pipefail

VERSION="2.10.1"
BASE_URL="https://github.com/tus/tusd/releases/download/v${VERSION}"
SERVER="tusd"
WITH_DIR="${TUS_UPLOAD_DIR:-}"
# --with-drill: also run the kill-9 TUS drill (tests/tus_drill.rs) after the
# suite — nightly-only: it uploads ~60 MiB across two real processes.
# --with-real: also run the real-data drill (tests/real_data.rs, issue #17)
# — a REAL ffmpeg-generated video through the engine; skipped loud where
# ffmpeg is absent.
WITH_DRILL=0
WITH_REAL=0
say() { echo "[tus-test] $*"; }
die() { echo "[tus-test] FAIL: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
  case "$1" in
    --server)
      [ $# -ge 2 ] || die "--server needs a value (tusd|tus-node)"
      case "$2" in
        tusd|tus-node) SERVER="$2" ;;
        *) die "unknown server: $2 (tusd|tus-node)" ;;
      esac
      shift ;;
    --with-drill) WITH_DRILL=1 ;;
    --with-real) WITH_REAL=1 ;;
    *) die "unknown flag: $1" ;;
  esac
  shift
done

if [ "$WITH_DRILL" = 1 ] && [ "$SERVER" != "tusd" ]; then
  die "--with-drill is tusd-canonical only (kill-9 drill semantics built vs tusd; see #33)"
fi

PY=python3
command -v python3 >/dev/null 2>&1 || PY=python
command -v "$PY" >/dev/null 2>&1 || die "no python for sha256/zip handling"

# --- 1. per-OS asset / server prep ------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
if [ "$SERVER" = "tus-node" ]; then
  command -v node >/dev/null 2>&1 || die "tus-node lane needs node"
  command -v npm  >/dev/null 2>&1 || die "tus-node lane needs npm"
  LANE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/tus-node-lane" && pwd)"
  if [ ! -d "$LANE_DIR/node_modules" ]; then
    say "installing pinned @tus/server + @tus/file-store (npm ci, lockfile integrity)"
    (cd "$LANE_DIR" && npm ci --no-audit --no-fund --loglevel=error) || die "npm ci"
  fi
  say "pinned server: @tus/server 2.4.7 + @tus/file-store 2.1.2 (node $(node --version))"
else
  case "$os" in
    Linux)  case "$arch" in x86_64) ASSET="tusd_linux_amd64.tar.gz";; aarch64|arm64) ASSET="tusd_linux_arm64.tar.gz";; *) die "unsupported linux arch: $arch";; esac ;;
    Darwin) case "$arch" in arm64) ASSET="tusd_darwin_arm64.zip";; x86_64) ASSET="tusd_darwin_amd64.zip";; *) die "unsupported mac arch: $arch";; esac ;;
    MINGW*|MSYS*|CYGWIN*) ASSET="tusd_windows_amd64.zip" ;;
    *) die "unsupported OS: $os" ;;
  esac
  say "pinned server: tusd ${VERSION} (${ASSET})"
fi

WORK="$(mktemp -d)"
cleanup() {
  if [ -n "${TUSD_PID:-}" ]; then
    kill "$TUSD_PID" >/dev/null 2>&1 || true
    # Windows/git-bash: wait for handles to release before rm (s3-test.sh lesson).
    for _ in $(seq 1 20); do
      kill -0 "$TUSD_PID" 2>/dev/null || break
      sleep 0.5
    done
    kill -9 "$TUSD_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK" "$TUS_DATA" 2>/dev/null || true
}
trap cleanup EXIT

# --- 2. random free port + temp upload dir ---------------------------------
TUS_PORT=$(( (RANDOM % 20000) + 20000 ))
TUS_DATA="${WITH_DIR:-$(mktemp -d)}"
mkdir -p "$TUS_DATA"

if [ "$SERVER" = "tus-node" ]; then
  say "booting @tus/server (port $TUS_PORT, data $TUS_DATA)"
  node "$LANE_DIR/server.mjs" "$TUS_DATA" "$TUS_PORT" > "$WORK/tus-node.log" 2>&1 &
  TUSD_PID=$!   # single kill-handle name reused by cleanup
else
  curl -sSL --retry 3 -o "$WORK/$ASSET" "$BASE_URL/$ASSET" || die "download $ASSET"
  curl -sSL --retry 3 -o "$WORK/$ASSET.sha256" "$BASE_URL/$ASSET.sha256" || die "download sha256 sidecar"
  actual="$(sha256sum "$WORK/$ASSET" | cut -d' ' -f1)"
  expected="$(cut -d' ' -f1 < "$WORK/$ASSET.sha256")"
  [ -n "$expected" ] && [ "$actual" = "$expected" ] \
    || die "sha256 mismatch for $ASSET (expected $expected, got $actual) — supply-chain guard"
  say "downloaded + sha256 verified"

  case "$ASSET" in
    *.tar.gz) tar -xzf "$WORK/$ASSET" -C "$WORK" ;;
    *.zip)    "$PY" -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$WORK/$ASSET" "$WORK" ;;
  esac
  TUSD="$(find "$WORK" -type f \( -name tusd -o -name tusd.exe \) | head -1)"
  [ -n "$TUSD" ] || die "tusd binary not found after extract"
  chmod +x "$TUSD" 2>/dev/null || true

  say "starting tusd (port $TUS_PORT)"
  "$TUSD" -host 127.0.0.1 -port "$TUS_PORT" -upload-dir "$TUS_DATA" \
    > "$WORK/tusd.log" 2>&1 &
  TUSD_PID=$!
fi

ready=0
for _ in $(seq 1 120); do
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 -X OPTIONS "http://127.0.0.1:${TUS_PORT}/files/" || true)"
  # tusd answers OPTIONS 200; @tus/server answers 204 — both carry Tus-Resumable.
  if [ "$code" = "200" ] || [ "$code" = "204" ]; then ready=1; break; fi
  kill -0 "$TUSD_PID" 2>/dev/null || break
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$WORK/${SERVER}.log" 2>/dev/null || tail -20 "$WORK/tus-node.log" 2>/dev/null || true; die "tus server ($SERVER) not ready"; }
say "tus server ($SERVER) ready"

# --- 3. env + tests ---------------------------------------------------------
command -v cargo >/dev/null 2>&1 \
  || { REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; [ -f "$REPO/scripts/rust-env.sh" ] && . "$REPO/scripts/rust-env.sh"; }
command -v cargo >/dev/null 2>&1 || die "cargo not on PATH"

export IUM_TUS_ENDPOINT="http://127.0.0.1:${TUS_PORT}/files/"
export IUM_TUS_SERVER="$SERVER"
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

say "running env-gated TUS suite (adapter_tus)"
cargo test -p ium-core --test adapter_tus

if [ "$WITH_DRILL" = 1 ]; then
  say "running kill-9 drill (tus_drill)"
  cargo test -p ium-core --test tus_drill -- --nocapture
fi

if [ "$WITH_REAL" = 1 ]; then
  say "running real-data drill vs $SERVER (real_data, issue #17; the TUS leg "\
       "loudly skips on the tus-node lane — its read-back needs tusd's GET extension, #33)"
  cargo test -p ium-core --test real_data -- --nocapture
fi

say "GREEN: TUS tests passed against $SERVER on ${os}/${arch}"
