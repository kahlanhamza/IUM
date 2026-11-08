#!/usr/bin/env bash
# issue55_wire_rehearsal.sh — live measurement-shape wire-facts rehearsal
# (issue #55, the M3 discipline: wire facts BEFORE Rust).
#
# Boots the pinned tusd v2.10.1 (same download + sha256 guard as tus-test.sh)
# and drives raw HTTP THROUGH the throttle proxy (scripts/throttle_proxy.py),
# capturing the facts the aggregate-vs-single drill (#50's open acceptance
# item) must be built on:
#   1. Location through a fronting proxy — does tusd build the Location from
#      the request's Host header (the proxy's, so the shaped wire carries the
#      whole session) or from its own -host/-port (a bypass that would cheat
#      the measurement)? THE architecture-deciding fact.
#   2. Per-connection cap shape (--per-conn-rate C): one PATCH rides ~C;
#      TWO concurrent PATCHes each still ride ~C — the aggregate multiplies
#      (K·C). This IS the prober's PerFlowCap vector, measured live.
#   3. Shared-bearer shape (--total-rate T): TWO concurrent PATCHes share
#      one T — the aggregate never exceeds the bearer. The BearerCap vector.
#   4. No-body requests (OPTIONS, creation POSTs) ride FREE in both modes —
#      the probe's creation phase must not eat the measurement budget.
#
# Usage: scripts/issue55_wire_rehearsal.sh   (no args; tusd only)

set -euo pipefail

VERSION="2.10.1"
BASE_URL="https://github.com/tus/tusd/releases/download/v${VERSION}"
say() { echo "[rehearsal55] $*"; }
die() { echo "[rehearsal55] FAIL: $*" >&2; exit 1; }

PY=python3
command -v python3 >/dev/null 2>&1 || PY=python
command -v curl >/dev/null 2>&1 || die "no curl"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROXY="$REPO/scripts/throttle_proxy.py"
[ -f "$PROXY" ] || die "throttle_proxy.py not found at $PROXY"

WORK="$(mktemp -d)"
TUS_DATA="$(mktemp -d)"
TUSD_PID=""
PROXY_PIDS=()
cleanup() {
  for pid in "${PROXY_PIDS[@]:-}"; do
    [ -n "$pid" ] && kill "$pid" >/dev/null 2>&1 || true
  done
  if [ -n "$TUSD_PID" ]; then
    kill "$TUSD_PID" >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do kill -0 "$TUSD_PID" 2>/dev/null || break; sleep 0.5; done
    kill -9 "$TUSD_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK" "$TUS_DATA" 2>/dev/null || true
}
trap cleanup EXIT

# --- tusd: cached download + sha256 guard (every run re-verifies) -----------
os="$(uname -s)"; arch="$(uname -m)"
case "$os" in
  Linux) case "$arch" in x86_64) ASSET="tusd_linux_amd64.tar.gz";; aarch64|arm64) ASSET="tusd_linux_arm64.tar.gz";; *) die "arch $arch";; esac ;;
  Darwin) case "$arch" in arm64) ASSET="tusd_darwin_arm64.zip";; x86_64) ASSET="tusd_darwin_amd64.zip";; *) die "arch $arch";; esac ;;
  MINGW*|MSYS*|CYGWIN*) ASSET="tusd_windows_amd64.zip" ;;
  *) die "unsupported OS: $os" ;;
esac
CACHE="/tmp/ium-tusd-${VERSION}"
mkdir -p "$CACHE"
TUSD="$CACHE/tusd"
if [ ! -x "$TUSD" ]; then
  say "pinned server: tusd ${VERSION} (${ASSET}) — downloading"
  curl -fsSL --retry 3 --retry-delay 2 -o "$CACHE/$ASSET" "$BASE_URL/$ASSET" || die "download $ASSET — upstream error"
  curl -fsSL --retry 3 --retry-delay 2 -o "$CACHE/$ASSET.sha256" "$BASE_URL/$ASSET.sha256" || die "download sha256 — upstream error"
  actual="$(sha256sum "$CACHE/$ASSET" | cut -d' ' -f1)"
  expected="$(cut -d' ' -f1 < "$CACHE/$ASSET.sha256")"
  echo "$expected" | grep -Eq '^[0-9a-fA-F]{64}$' || die "sha256 sidecar is not a digest — upstream served an error page?"
  [ "$actual" = "$expected" ] || die "sha256 mismatch (expected $expected, got $actual)"
  say "sha256 verified"
  case "$ASSET" in
    *.tar.gz) tar -xzf "$CACHE/$ASSET" -C "$CACHE" ;;
    *.zip) "$PY" -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$CACHE/$ASSET" "$CACHE" ;;
  esac
  FOUND="$(find "$CACHE" -type f \( -name tusd -o -name tusd.exe \) | head -1)"
  [ -n "$FOUND" ] || die "tusd binary not found after extract"
  mv "$FOUND" "$TUSD"; chmod +x "$TUSD" 2>/dev/null || true
else
  say "pinned server: tusd ${VERSION} (cached at $CACHE)"
fi

TUS_PORT=$(( (RANDOM % 20000) + 20000 ))
say "starting tusd (port $TUS_PORT)"
"$TUSD" -host 127.0.0.1 -port "$TUS_PORT" -upload-dir "$TUS_DATA" > "$WORK/tusd.log" 2>&1 &
TUSD_PID=$!
ready=0
for _ in $(seq 1 120); do
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 -X OPTIONS "http://127.0.0.1:${TUS_PORT}/files/" || true)"
  [ "$code" = "200" ] && { ready=1; break; }
  kill -0 "$TUSD_PID" 2>/dev/null || break
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$WORK/tusd.log" || true; die "tusd not ready"; }
say "tusd ready"

start_proxy() { # $1 = listen port, rest = proxy flags
  local lport="$1"; shift
  "$PY" "$PROXY" "$lport" 127.0.0.1 "$TUS_PORT" --victim-part none "$@" \
    > "$WORK/proxy-$lport.log" 2>&1 &
  PROXY_PIDS+=("$!")
  for _ in $(seq 1 40); do
    curl -s -o /dev/null --max-time 1 -X OPTIONS "http://127.0.0.1:${lport}/files/" && return 0
    sleep 0.25
  done
  die "proxy on $lport not ready"
}
free_port() { echo $(( (RANDOM % 20000) + 40000 )); }

# --- three shaped wires, all fronting the same tusd --------------------------
P_PASSTHROUGH="$(free_port)"; start_proxy "$P_PASSTHROUGH"
P_PERCONN="$(free_port)";     start_proxy "$P_PERCONN" --per-conn-rate 65536
P_BEARER="$(free_port)";      start_proxy "$P_BEARER" --total-rate 65536
say "proxies ready: passthrough=$P_PASSTHROUGH per-conn=$P_PERCONN bearer=$P_BEARER"
export IUM_PROXY_PASSTHROUGH="$P_PASSTHROUGH" IUM_PROXY_PERCONN="$P_PERCONN" \
       IUM_PROXY_BEARER="$P_BEARER"

IUM_TUS_PORT="$TUS_PORT" IUM_WORK="$WORK" "$PY" <<'DRIVER'
import http.client, os, time, threading

TUS_PORT = int(os.environ["IUM_TUS_PORT"])
WORK = os.environ["IUM_WORK"]
PIECE = 256 * 1024
C = 64 * 1024  # the drill's per-flow / bearer rate

def req(port, method, path, headers=None, body=None):
    t0 = time.monotonic()
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=120)
    c.request(method, path, body=body, headers=headers or {})
    r = c.getresponse()
    data = {
        "status": r.status,
        "location": r.getheader("Location"),
        "offset": r.getheader("Upload-Offset"),
        "ext": r.getheader("Tus-Extension"),
        "secs": time.monotonic() - t0,
        "body": r.read()[:200].decode("utf-8", "replace"),
    }
    c.close()
    return data

print("=" * 72)
print("WIRE FACTS — the measurement shapes through the proxy (issue #55)")
print("=" * 72)

# --- fact 1: Location through the proxy (passthrough) -----------------------
P1 = int(os.environ["IUM_PROXY_PASSTHROUGH"])

TR = {"Tus-Resumable": "1.0.0"}
r = req(P1, "OPTIONS", "/files/")
print(f"1a. OPTIONS via proxy -> {r['status']}  ({r['secs']*1000:.0f} ms, no body -> free)")
print(f"    Tus-Extension: {r['ext']}")
r = req(P1, "POST", "/files/", {**TR, "Upload-Length": str(PIECE)})
loc = r["location"] or ""
print(f"1b. POST creation via proxy -> {r['status']}  ({r['secs']*1000:.0f} ms)")
print(f"    Location: {loc}")
loc_host = loc.split("//")[-1].split("/")[0] if loc.startswith("http") else "(relative)"
proxy_host = f"127.0.0.1:{P1}"
tusd_host = f"127.0.0.1:{TUS_PORT}"
if loc_host == proxy_host:
    print(f"    => FACT 1: tusd honors the request Host — Location points at the "
          f"PROXY ({proxy_host}); the shaped wire carries the whole session.")
elif loc_host == tusd_host:
    print(f"    => FACT 1 (BYPASS): Location points at tusd itself ({tusd_host}) — "
          f"the adapter would ride AROUND the shaped wire; the drill needs "
          f"tusd -behind-proxy + X-Forwarded-Host.")
else:
    print(f"    => FACT 1 (UNEXPECTED): Location host = {loc_host}")
session = loc

# --- fact 2: per-connection cap shape ---------------------------------------
P2 = int(os.environ["IUM_PROXY_PERCONN"])
body = bytes([0x5A]) * PIECE
def patch(port, sess, results, idx):
    t0 = time.monotonic()
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=120)
    c.request("PATCH", sess, body=body, headers={**TR,
              "Upload-Offset": "0", "Content-Type": "application/offset+octet-stream"})
    r = c.getresponse(); r.read(); c.close()
    secs = time.monotonic() - t0
    results[idx] = (r.status, secs)

mk = lambda port: req(port, "POST", "/files/", {**TR, "Upload-Length": str(PIECE)})["location"]
s = mk(P2)
r = req(P2, "PATCH", s, {**TR, "Upload-Offset": "0",
        "Content-Type": "application/offset+octet-stream"}, body)
rate = PIECE / r["secs"]
print(f"2a. per-conn: single PATCH {PIECE}B -> {r['status']} in {r['secs']:.2f}s "
      f"= {rate/1024:.0f} KiB/s (cap C=64)")
res = {}
th = [threading.Thread(target=patch, args=(P2, mk(P2), res, 0)),
      threading.Thread(target=patch, args=(P2, mk(P2), res, 1))]
t0 = time.monotonic()
for t in th: t.start()
for t in th: t.join()
wall = time.monotonic() - t0
rates = [PIECE / secs / 1024 for (_st, secs) in res.values()]
print(f"2b. per-conn: TWO concurrent PATCHes (own sessions) -> statuses "
      f"{[res[i][0] for i in range(2)]}, per-flow {[f'{x:.0f}' for x in rates]} KiB/s, "
      f"aggregate {sum(rates):.0f} KiB/s in {wall:.2f}s (K·C = 128)")
print(f"    => FACT 2: aggregate {'MULTIPLIES (K·C)' if sum(rates) > 1.5*64 else 'DID NOT multiply'}")

# --- fact 3: shared-bearer shape --------------------------------------------
P3 = int(os.environ["IUM_PROXY_BEARER"])
body = bytes([0x5A]) * PIECE
s = mk(P3)
r = req(P3, "PATCH", s, {**TR, "Upload-Offset": "0",
        "Content-Type": "application/offset+octet-stream"}, body)
rate = PIECE / r["secs"]
print(f"3a. bearer: single PATCH {PIECE}B -> {r['status']} in {r['secs']:.2f}s "
      f"= {rate/1024:.0f} KiB/s (bearer T=64)")
s2 = mk(P3)
s3 = mk(P3)
res = {}
th = [threading.Thread(target=patch, args=(P3, s2, res, 0)),
      threading.Thread(target=patch, args=(P3, s3, res, 1))]
t0 = time.monotonic()
for t in th: t.start()
for t in th: t.join()
wall = time.monotonic() - t0
rates = [PIECE / secs / 1024 for (_st, secs) in res.values()]
print(f"3b. bearer: TWO concurrent PATCHes (two sessions) -> statuses "
      f"{[res[i][0] for i in range(2)]}, per-flow {[f'{x:.0f}' for x in rates]} KiB/s, "
      f"aggregate {sum(rates):.0f} KiB/s in {wall:.2f}s (T = 64)")
print(f"    => FACT 3: aggregate {'STAYS AT THE BEARER' if sum(rates) < 1.5*64 else 'EXCEEDED THE BEARER (bucket leak!)'}")

print("=" * 72)
print("WIRE FACTS captured — pin on issue #55, then build the drill.")
print("=" * 72)
DRIVER

say "GREEN: measurement-shape rehearsal complete vs tusd ${VERSION} (${os}/${arch})"
