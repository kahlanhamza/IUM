#!/usr/bin/env bash
# RED repro for #89 — proves the HEAD behavior against an upstream that 504s.
#
# The local server mimics the CI brown-out seen in nightly run 42468487548:
# every request answered HTTP 504 with an HTML body. We then run the EXACT
# fetch+guard lines from scripts/tus-test.sh at HEAD (L157–163) and show:
#   1. curl exits 0 (no || die fires) even though the body is an error page,
#   2. the HTML lands inside the .sha256 sidecar,
#   3. the guard dies with the cryptic "sha256 mismatch (expected <html>…)".
set -u
cd "$(dirname "$0")/.." || exit 1

PORT=18971
LOG=/tmp/red89_server.log

python3 - "$PORT" >"$LOG" 2>&1 <<'PY' &
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

class H(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"<html><body><h1>504 Gateway Time-out</h1>The server didn't respond in time.</body></html>"
        self.send_response(504)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass

HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PY
SRV=$!
trap 'kill $SRV 2>/dev/null' EXIT
for i in $(seq 1 50); do
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/" ; then break; fi
  sleep 0.1
done

WORK=$(mktemp -d)
ASSET=fake_artifact.tar.gz
BASE_URL="http://127.0.0.1:$PORT"

echo "=== HEAD fetch lines (verbatim from scripts/tus-test.sh L157–163) ==="
curl -sSL --retry 3 -o "$WORK/$ASSET" "$BASE_URL/$ASSET" || echo "DIE: download $ASSET"
curl -sSL --retry 3 -o "$WORK/$ASSET.sha256" "$BASE_URL/$ASSET.sha256" || echo "DIE: download sha256 sidecar"
actual="$(sha256sum "$WORK/$ASSET" | cut -d' ' -f1)"
expected="$(cut -d' ' -f1 < "$WORK/$ASSET.sha256")"
echo "abcdef0123456789 $ASSET" > /dev/null  # (tarball bytes are also 504 HTML here; hash computed above)
[ -n "$expected" ] && [ "$actual" = "$expected" ] \
  || echo "[tus-test] FAIL: sha256 mismatch for $ASSET (expected $expected, got $actual) — supply-chain guard"
echo
echo "=== Forensics ==="
echo "sidecar bytes: $(head -c 60 "$WORK/$ASSET.sha256")"
echo "curl exit codes: tarball=$? (last op) — both curls above exited 0 or the DIE lines would have printed"
