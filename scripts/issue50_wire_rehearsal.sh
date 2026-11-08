#!/usr/bin/env bash
# issue50_wire_rehearsal.sh — live tus concatenation wire-facts rehearsal
# (issue #50, M3 discipline: wire facts BEFORE Rust).
#
# Boots the pinned tusd v2.10.1 (same download + sha256 guard as tus-test.sh)
# on a random port and hand-drives the concatenation extension with raw HTTP,
# capturing every request/response line as the evidence block for issue #50:
#   1. OPTIONS discovery        — Tus-Extension carries "concatenation"?
#   2. partial creation         — POST + Upload-Concat: partial + Upload-Length
#   3. concurrent PATCHes       — N partials advanced in parallel
#   4. HEAD partial             — per-session offset truth
#   5. final creation           — POST + Upload-Concat: final;<u1> <u2> ...
#                                 (with and without Upload-Length)
#   6. HEAD final               — materialization offset (eager vs async)
#   7. GET final                — byte-exact concatenation
#   8. error shapes             — bogus partial URL, PATCH on final,
#                                 incomplete partial in the list, missing
#                                 Upload-Length on partial, relative URL list
#
# Usage: scripts/issue50_wire_rehearsal.sh   (no args; tusd only — the
# @tus/server lane has NO concatenation, that delta is already recorded).

set -euo pipefail

VERSION="2.10.1"
BASE_URL="https://github.com/tus/tusd/releases/download/v${VERSION}"
say() { echo "[rehearsal] $*"; }
die() { echo "[rehearsal] FAIL: $*" >&2; exit 1; }

PY=python3
command -v python3 >/dev/null 2>&1 || PY=python
command -v "$PY" >/dev/null 2>&1 || die "no python"
command -v curl >/dev/null 2>&1 || die "no curl"

WORK="$(mktemp -d)"
cleanup() {
  if [ -n "${TUSD_PID:-}" ]; then
    kill "$TUSD_PID" >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do kill -0 "$TUSD_PID" 2>/dev/null || break; sleep 0.5; done
    kill -9 "$TUSD_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK" "$TUS_DATA" 2>/dev/null || true
}
trap cleanup EXIT

os="$(uname -s)"; arch="$(uname -m)"
case "$os" in
  Linux) case "$arch" in x86_64) ASSET="tusd_linux_amd64.tar.gz";; aarch64|arm64) ASSET="tusd_linux_arm64.tar.gz";; *) die "arch $arch";; esac ;;
  Darwin) case "$arch" in arm64) ASSET="tusd_darwin_arm64.zip";; x86_64) ASSET="tusd_darwin_amd64.zip";; *) die "arch $arch";; esac ;;
  MINGW*|MSYS*|CYGWIN*) ASSET="tusd_windows_amd64.zip" ;;
  *) die "unsupported OS: $os" ;;
esac
say "pinned server: tusd ${VERSION} (${ASSET})"
curl -fsSL --retry 3 --retry-delay 2 -o "$WORK/$ASSET" "$BASE_URL/$ASSET" || die "download $ASSET — upstream error"
curl -fsSL --retry 3 --retry-delay 2 -o "$WORK/$ASSET.sha256" "$BASE_URL/$ASSET.sha256" || die "download sha256 — upstream error"
actual="$(sha256sum "$WORK/$ASSET" | cut -d' ' -f1)"
expected="$(cut -d' ' -f1 < "$WORK/$ASSET.sha256")"
echo "$expected" | grep -Eq '^[0-9a-fA-F]{64}$' || die "sha256 sidecar is not a digest — upstream served an error page?"
[ "$actual" = "$expected" ] || die "sha256 mismatch (expected $expected, got $actual)"
say "sha256 verified"
case "$ASSET" in
  *.tar.gz) tar -xzf "$WORK/$ASSET" -C "$WORK" ;;
  *.zip) "$PY" -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$WORK/$ASSET" "$WORK" ;;
esac
TUSD="$(find "$WORK" -type f \( -name tusd -o -name tusd.exe \) | head -1)"
[ -n "$TUSD" ] || die "tusd binary not found"
chmod +x "$TUSD" 2>/dev/null || true

PORT=$(( (RANDOM % 20000) + 20000 ))
TUS_DATA="$(mktemp -d)"
say "starting tusd (port $PORT)"
"$TUSD" -host 127.0.0.1 -port "$PORT" -upload-dir "$TUS_DATA" > "$WORK/tusd.log" 2>&1 &
TUSD_PID=$!
ready=0
for _ in $(seq 1 120); do
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 -X OPTIONS "http://127.0.0.1:${PORT}/files/" || true)"
  [ "$code" = "200" ] && { ready=1; break; }
  kill -0 "$TUSD_PID" 2>/dev/null || break
  sleep 0.5
done
[ "$ready" = 1 ] || { tail -20 "$WORK/tusd.log" || true; die "tusd not ready"; }
say "tusd ready"

IUM_PORT="$PORT" "$PY" <<'DRIVER'
import http.client, os, sys, itertools, concurrent.futures

PORT = int(os.environ["IUM_PORT"])
HOST = "127.0.0.1"
BASE = f"http://{HOST}:{PORT}"
N = 3            # partial sessions
PIECE = 1 << 20  # 1 MiB each

conn = lambda: http.client.HTTPConnection(HOST, PORT, timeout=30)

def req(method, path, headers=None, body=None):
    c = conn()
    c.request(method, path, body=body, headers=headers or {})
    r = c.getresponse()
    data = {
        "status": r.status,
        "location": r.getheader("Location"),
        "offset": r.getheader("Upload-Offset"),
        "concat": r.getheader("Upload-Concat"),
        "extension": r.getheader("Tus-Extension"),
        "version": r.getheader("Tus-Version"),
        "maxsize": r.getheader("Tus-Max-Size"),
        "body": r.read()[:300].decode("utf-8", "replace"),
    }
    c.close()
    return data

print("=" * 72)
print("WIRE FACTS — tusd 2.10.1 concatenation (live rehearsal, issue #50)")
print("=" * 72)

# 1. OPTIONS discovery -------------------------------------------------------
o = req("OPTIONS", "/files/")
print(f"1. OPTIONS /files/ -> {o['status']}  Tus-Version={o['version']}")
print(f"   Tus-Extension: {o['extension']}")
has_concat = o["extension"] and "concatenation" in o["extension"]
print(f"   => concatenation advertised: {has_concat}")

TR = {"Tus-Resumable": "1.0.0"}

# 2. partial creation xN -----------------------------------------------------
partials = []
for i in range(N):
    r = req("POST", "/files/", {**TR, "Upload-Concat": "partial",
                                "Upload-Length": str(PIECE)})
    print(f"2. POST partial #{i} (Upload-Concat: partial, Upload-Length {PIECE}) "
          f"-> {r['status']}  Location={r['location']}  Upload-Concat-echo={r['concat']}")
    assert r["status"] == 201, r
    partials.append(r["location"])

# 3. concurrent PATCHes ------------------------------------------------------
payloads = [bytes([0xA0 + i]) * PIECE for i in range(N)]
def push(args):
    i, loc, data = args
    r = req("PATCH", loc, {**TR, "Upload-Offset": "0",
                           "Content-Type": "application/offset+octet-stream",
                           "Content-Length": str(len(data))}, data)
    return i, r
with concurrent.futures.ThreadPoolExecutor(max_workers=N) as ex:
    results = dict(ex.map(push, [(i, partials[i], payloads[i]) for i in range(N)]))
for i in range(N):
    r = results[i]
    print(f"3. PATCH partial #{i} (1 MiB, concurrent) -> {r['status']}  Upload-Offset={r['offset']}")
    assert r["status"] == 204 and r["offset"] == str(PIECE), r

# 4. HEAD partial ------------------------------------------------------------
for i in range(N):
    r = req("HEAD", partials[i], TR)
    print(f"4. HEAD partial #{i} -> {r['status']}  Upload-Offset={r['offset']}")
    assert r["status"] == 200 and r["offset"] == str(PIECE), r

# 5. final creation — WITH Upload-Length first, then WITHOUT -----------------
final_list = "final;" + " ".join(partials)
r_with = req("POST", "/files/", {**TR, "Upload-Concat": final_list,
                                 "Upload-Length": str(N * PIECE)})
print(f"5a. POST final WITH Upload-Length -> {r_with['status']}  "
      f"Location={r_with['location']}  Concat-echo={r_with['concat']}")
r_without = req("POST", "/files/", {**TR, "Upload-Concat": final_list})
print(f"5b. POST final WITHOUT Upload-Length -> {r_without['status']}  "
      f"Location={r_without['location']}  (same partial list reused!)")
FINAL = r_with["location"]
assert r_with["status"] == 201 and FINAL, r_with

# 6. HEAD final — eager or async? --------------------------------------------
r = req("HEAD", FINAL, TR)
print(f"6. HEAD final (immediately) -> {r['status']}  Upload-Offset={r['offset']} "
      f"(expected {N*PIECE} if materialization is eager)")

# 7. GET final — byte-exact --------------------------------------------------
r = req("GET", FINAL)
body = r["body"]
print(f"7. GET final -> {r['status']}  (first bytes of body captured only; "
      f"full compare below)")
c = conn()
c.request("GET", FINAL)
resp = c.getresponse()
full = resp.read(); c.close()
expect = b"".join(payloads)
print(f"   body len={len(full)}  byte-exact == concat(partials): {full == expect}")
assert resp.status == 200 and full == expect

# 8. error shapes ------------------------------------------------------------
# 8a. final with a bogus partial URL
r = req("POST", "/files/", {**TR, "Upload-Concat": "final;/files/does-not-exist",
                            "Upload-Length": str(PIECE)})
print(f"8a. POST final with bogus partial URL -> {r['status']}  body={r['body'][:120]!r}")
# 8b. PATCH on the FINAL upload
r = req("PATCH", FINAL, {**TR, "Upload-Offset": "0",
                         "Content-Type": "application/offset+octet-stream",
                         "Content-Length": "4"}, b"XXXX")
print(f"8b. PATCH on the final upload -> {r['status']}  body={r['body'][:120]!r}")
# 8c. final referencing an INCOMPLETE partial
r = req("POST", "/files/", {**TR, "Upload-Concat": "partial",
                            "Upload-Length": str(PIECE)})
inc = r["location"]
r = req("PATCH", inc, {**TR, "Upload-Offset": "0",
                       "Content-Type": "application/offset+octet-stream",
                       "Content-Length": "16"}, b"B" * 16)
print(f"    (seeded incomplete partial {inc} at 16/{PIECE} bytes -> {r['status']})")
r = req("POST", "/files/", {**TR, "Upload-Concat": f"final;{inc}",
                            "Upload-Length": str(PIECE)})
print(f"8c. POST final with INCOMPLETE partial -> {r['status']}  body={r['body'][:120]!r}")
if r["location"]:
    h = req("HEAD", r["location"], TR)
    print(f"    HEAD that final -> {h['status']}  Upload-Offset={h['offset']}")
# 8d. partial creation WITHOUT Upload-Length
r = req("POST", "/files/", {**TR, "Upload-Concat": "partial"})
print(f"8d. POST partial WITHOUT Upload-Length -> {r['status']}  body={r['body'][:120]!r}")
# 8e. relative (path-only) URLs in the final list
r = req("POST", "/files/", {**TR, "Upload-Concat": "final;" + " ".join(
        p.split(f"{HOST}:{PORT}")[-1] if f"{HOST}:{PORT}" in p else p
        for p in partials), "Upload-Length": str(N * PIECE)})
print(f"8e. POST final with RELATIVE partial URLs -> {r['status']}  "
      f"Location={r['location']}  body={r['body'][:120]!r}")

print("=" * 72)
print("WIRE FACTS captured — pin on issue #50, then build the adapter.")
print("=" * 72)
DRIVER

say "GREEN: rehearsal complete vs tusd ${VERSION} (${os}/${arch})"
