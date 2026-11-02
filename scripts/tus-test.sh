#!/usr/bin/env bash
# tus-test.sh — the 3-OS TUS test harness (issue #10).
#
# Downloads the PINNED tusd binary for this runner (tus/tusd v2.10.1 — the
# reference TUS 1.0.0 server, single binary per OS from GitHub Releases with
# sha256 sidecars), boots it on a random free port with a temp upload dir,
# exports IUM_TUS_ENDPOINT, runs the env-gated TUS suite, then kills the
# server and removes the temp dir (TUS has no multipart-orphan equivalent —
# the upload dir IS the ephemeral state).
#
# Usage: scripts/tus-test.sh
# Requires: bash, curl, tar/unzip, python3|python, cargo (already set up).

set -euo pipefail

VERSION="2.10.1"
BASE_URL="https://github.com/tus/tusd/releases/download/v${VERSION}"
WITH_DIR="${TUS_UPLOAD_DIR:-}"
# --with-drill: also run the kill-9 TUS drill (tests/tus_drill.rs) after the
# suite — nightly-only: it uploads ~60 MiB across two real processes.
WITH_DRILL=0
[ "${1:-}" = "--with-drill" ] && WITH_DRILL=1

say() { echo "[tus-test] $*"; }
die() { echo "[tus-test] FAIL: $*" >&2; exit 1; }

PY=python3
command -v python3 >/dev/null 2>&1 || PY=python
command -v "$PY" >/dev/null 2>&1 || die "no python for sha256/zip handling"

# --- 1. per-OS asset --------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  case "$arch" in x86_64) ASSET="tusd_linux_amd64.tar.gz";; aarch64|arm64) ASSET="tusd_linux_arm64.tar.gz";; *) die "unsupported linux arch: $arch";; esac ;;
  Darwin) case "$arch" in arm64) ASSET="tusd_darwin_arm64.zip";; x86_64) ASSET="tusd_darwin_amd64.zip";; *) die "unsupported mac arch: $arch";; esac ;;
  MINGW*|MSYS*|CYGWIN*) ASSET="tusd_windows_amd64.zip" ;;
  *) die "unsupported OS: $os" ;;
esac
say "pinned server: tusd ${VERSION} (${ASSET})"

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

# --- 2. random free port + temp upload dir ---------------------------------
TUS_PORT=$(( (RANDOM % 20000) + 20000 ))
TUS_DATA="${WITH_DIR:-$(mktemp -d)}"
mkdir -p "$TUS_DATA"

say "starting tusd (port $TUS_PORT)"
"$TUSD" -host 127.0.0.1 -port "$TUS_PORT" -upload-dir "$TUS_DATA" \
  > "$WORK/tusd.log" 2>&1 &
TUSD_PID=$!

ready=0
for _ in $(seq 1 120); do
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 -X OPTIONS "http://127.0.0.1:${TUS_PORT}/files/" || true)"
  if [ "$code" = "200" ]; then ready=1; break; fi
  kill -0 "$TUSD_PID" 2>/dev/null || break
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$WORK/tusd.log" >&2; die "tusd not ready"; }
say "tusd ready"

# --- 3. env + tests ---------------------------------------------------------
command -v cargo >/dev/null 2>&1 \
  || { REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; [ -f "$REPO/scripts/rust-env.sh" ] && . "$REPO/scripts/rust-env.sh"; }
command -v cargo >/dev/null 2>&1 || die "cargo not on PATH"

export IUM_TUS_ENDPOINT="http://127.0.0.1:${TUS_PORT}/files/"
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

say "running env-gated TUS suite (adapter_tus)"
cargo test -p ium-core --test adapter_tus

if [ "$WITH_DRILL" = 1 ]; then
  say "running kill-9 drill (tus_drill)"
  cargo test -p ium-core --test tus_drill -- --nocapture
fi

say "GREEN: TUS tests passed against tusd ${VERSION} on ${os}/${arch}"
