#!/usr/bin/env bash
# GREEN repro for #89 — the hardened fetch path under the two adversarial cases.
#
#   Case A: server answers 504 + HTML for everything (the nightly #12 brown-out).
#           Expect: curl retries (4 attempts), exits non-zero, saves NOTHING,
#           die fires with "download … — upstream error (HTTP 4xx/5xx?)".
#   Case B: tarball 200 + healthy bytes, sidecar 200 + HTML body (captive portal
#           / error-page-with-200). Expect: the digest-format guard dies with
#           "… is not a 64-hex digest — upstream served an error page?".
#   Case C: healthy end-to-end — tarball + true sidecar. Expect: "downloaded + sha256 verified".
set -u
cd "$(dirname "$0")/.." || exit 1

# ---- Case A/B/C servers -------------------------------------------------
PORT_A=18981; PORT_B=18982; PORT_C=18983
python3 - "$PORT_A" > /tmp/grn89_a.log 2>&1 <<'PY' &   # always 504
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    hits = 0
    def do_GET(self):
        H.hits += 1
        b = b"<html><body><h1>504 Gateway Time-out</h1></body></html>"
        self.send_response(504); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PY
PA=$!
python3 - "$PORT_B" > /tmp/grn89_b.log 2>&1 <<'PY' &   # 200 HTML sidecar, real-ish tarball
import sys, io, tarfile
from http.server import BaseHTTPRequestHandler, HTTPServer
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w:gz") as t:
    ti = tarfile.TarInfo("x"); ti.size = 0
    import io as _io; t.addfile(ti, _io.BytesIO(b""))
TARBALL = buf.getvalue()
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.endswith(".sha256"):
            b = b"<html><body><h1>502 Bad Gateway</h1>proxied</body></html>"
            self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
        else:
            self.send_response(200); self.send_header("Content-Length", str(len(TARBALL))); self.end_headers(); self.wfile.write(TARBALL)
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PY
PB=$!
python3 - "$PORT_C" > /tmp/grn89_c.log 2>&1 <<'PY' &   # healthy
import sys, io, tarfile, hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w:gz") as t:
    ti = tarfile.TarInfo("x"); ti.size = 0
    import io as _io; t.addfile(ti, _io.BytesIO(b""))
TARBALL = buf.getvalue()
DIGEST = hashlib.sha256(TARBALL).hexdigest()
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        b = (DIGEST + "  artifact.tar.gz\n").encode() if self.path.endswith(".sha256") else TARBALL
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PY
PC=$!
trap 'kill $PA $PB $PC 2>/dev/null' EXIT
for p in $PORT_A $PORT_B $PORT_C; do until curl -s -o /dev/null "http://127.0.0.1:$p/"; do sleep 0.1; done; done

die() { echo "[tus-test] FAIL: $*" >&2; exit 1; }
say() { echo "[tus-test] $*"; }

run_case() { # $1=port $2=label  (subshell: die must exit only the case)
  ( local BASE_URL="http://127.0.0.1:$1" WORK ASSET
  ASSET=artifact.tar.gz; WORK=$(mktemp -d)
  echo "=== Case $2 ==="
  # verbatim hardened lines from scripts/tus-test.sh L159–166
  curl -fsSL --retry 3 --retry-delay 1 -o "$WORK/$ASSET" "$BASE_URL/$ASSET" || die "download $ASSET — upstream error (HTTP 4xx/5xx?)"
  curl -fsSL --retry 3 --retry-delay 1 -o "$WORK/$ASSET.sha256" "$BASE_URL/$ASSET.sha256" || die "download sha256 sidecar — upstream error (HTTP 4xx/5xx?)"
  actual="$(sha256sum "$WORK/$ASSET" | cut -d' ' -f1)"
  expected="$(cut -d' ' -f1 < "$WORK/$ASSET.sha256")"
  echo "$expected" | grep -Eq '^[0-9a-fA-F]{64}$' \
    || die "sha256 sidecar for $ASSET is not a 64-hex digest — upstream served an error page? (supply-chain guard)"
  [ "$actual" = "$expected" ] \
    || die "sha256 mismatch for $ASSET (expected $expected, got $actual) — supply-chain guard"
  say "downloaded + sha256 verified"
  echo )
}

run_case $PORT_A A   # the four `curl: (22)` lines below = 1 attempt + 3 retries, then die
run_case $PORT_B B
run_case $PORT_C C
echo "bash -n both scripts:"; bash -n scripts/tus-test.sh && echo "  tus-test.sh: syntax OK"; bash -n scripts/s3-test.sh && echo "  s3-test.sh: syntax OK"
