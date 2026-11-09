#!/usr/bin/env bash
# longhaul.sh — the long-horizon & resource-honesty leg (issue #34).
#
# Run UNDER scripts/s3-test.sh --with-longhaul FILE [RATE_Bps [TIMEBOX_SECS]]:
# the pinned SeaweedFS is booted, the bucket exists and IUM_S3_* are exported.
# This driver runs the `longhaul` scenario (ium-core/src/bin/realworld_stress.rs)
# through the uniform-throttle proxy:
#
#   RATE_Bps    the whole uplink choked to this many bytes/s
#               (default 512 — the owner's measured reality; 30 MB ≈ 17 h)
#   TIMEBOX     wall-clock seconds the leg may run in THIS box
#               (default 90 s in-session; the overnight cloud run sets its own)
#
# The leg ends HONESTLY either way:
#   * Done inside the timebox (a short leg is still a leg), or
#   * PAUSED at the timebox (never canceled — resumable), then a FRESH manager
#     over the same journal replays and resumes to Done at full speed.
# Server truth is the read-back sha256. Evidence: rea-lab/longhaul/evidence-*.md
# with the [longhaul][persist] telemetry lines (frontier / journal bytes / RSS).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT="$REPO/rea-lab/longhaul"
STAMP="$(date +%Y%m%d-%H%M%S)"
RUN="$OUT/run-$STAMP"
mkdir -p "$RUN/journal"
EVID="$OUT/evidence-$STAMP.md"

say() { echo "[longhaul] $*"; }
die() { echo "[longhaul] FAIL: $*" >&2; exit 1; }

[ -n "${IUM_S3_ENDPOINT:-}" ] || die "run under s3-test.sh --with-longhaul (IUM_S3_ENDPOINT missing)"

FILE="${1:?usage: longhaul.sh FILE [RATE_Bps] [TIMEBOX_SECS]}"
[ -f "$FILE" ] || die "asset not found: $FILE"
RATE="${2:-512}"
TIMEBOX="${3:-90}"
PY3="${PY:-python3}"

sha_of() { sha256sum "$1" | cut -d' ' -f1; }

BASE="$IUM_S3_ENDPOINT"
AK="${IUM_S3_ACCESS_KEY:?}"
SK="${IUM_S3_SECRET_KEY:?}"
REGION="${IUM_S3_REGION:-us-east-1}"

s3_status() { # METHOD URL → HTTP status
  "$PY3" - "$1" "$2" <<'EOF'
import sys
sys.path.insert(0, "scripts")
import os, s3curl
st, _, _ = s3curl.request(sys.argv[1], sys.argv[2],
                          os.environ["IUM_S3_ACCESS_KEY"],
                          os.environ["IUM_S3_SECRET_KEY"],
                          os.environ.get("IUM_S3_REGION", "us-east-1"))
print(st)
EOF
}

s3_get_to() { # URL OUTFILE → status; body written to OUTFILE
  "$PY3" - "$1" "$2" <<'EOF'
import sys
sys.path.insert(0, "scripts")
import os, s3curl
st, body, _ = s3curl.request("GET", sys.argv[1],
                             os.environ["IUM_S3_ACCESS_KEY"],
                             os.environ["IUM_S3_SECRET_KEY"],
                             os.environ.get("IUM_S3_REGION", "us-east-1"))
if str(st).startswith("2"):
    open(sys.argv[2], "wb").write(body)
print(st)
EOF
}

{
  echo "# longhaul evidence $STAMP"
  echo "- asset: $(basename "$FILE") — $(stat -c%s "$FILE") bytes — sha256 $(sha_of "$FILE")"
  echo "- model: whole uplink @ ${RATE} B/s (proxy, --victim-part all); timebox ${TIMEBOX}s"
  echo "- overnight reference: 30 MB @ 512 B/s ≈ 17 h — same scenario, --seconds sized for the box"
} > "$EVID"

# --- the throttled lane -----------------------------------------------------
SRV_HOSTport="${BASE#http://}"          # 127.0.0.1:PORT/bucket
SRV_HOST="${SRV_HOSTport%%:*}"
SRV_PORT="$(echo "${SRV_HOSTport%%/*}" | cut -d: -f2)"
LH_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
# The engine's endpoint must point THROUGH the proxy (the paced lane), keeping
# the bucket path: the first longhaul runs proved this the hard way — a key on
# the direct URL uploads unthrottled (no UNIFORM lines in the proxy log).
BUCKET_PATH="${BASE#http://*/}"         # bucket
KEY="http://127.0.0.1:${LH_PORT}/${BUCKET_PATH}/longhaul/$(basename "$FILE")"   # engine's lane (paced)
DIRECT_KEY="$BASE/longhaul/$(basename "$FILE")"                                # server truth (read-back)

"$PY3" "$SCRIPT_DIR/throttle_proxy.py" "$LH_PORT" "$SRV_HOST" "$SRV_PORT" \
  --victim-part all --victim-rate "$RATE" --victim-seconds 100000 \
  > "$RUN/longhaul-proxy.log" 2>&1 &
PROXY_PID=$!
trap 'kill "$PROXY_PID" 2>/dev/null || true' EXIT
sleep 1
say "proxy up on $LH_PORT → $SRV_HOST:$SRV_PORT (uniform ${RATE} B/s)"

# --- the leg ----------------------------------------------------------------
say "running longhaul: $(basename "$FILE") @ ${RATE} B/s, timebox ${TIMEBOX}s"
OUT_LH="$(cargo run -q -p ium-core --bin realworld_stress -- \
  longhaul --journal "$RUN/journal" --file "$FILE" --endpoint "$KEY" \
  --rate "$RATE" --seconds "$TIMEBOX" 2>&1)" || {
  echo "$OUT_LH" | tail -30 >&2; die "longhaul bin failed";
}
# Persist lines + EVIDENCE into the artifact (telemetry is the deliverable).
# POSIX grep, NOT rg: the GitHub ubuntu runner ships no ripgrep, and a
# missing post-processing toy killed a GREEN 25-minute leg at dispatch
# 42692635806 (`rg: command not found` → pipefail → the step died AFTER
# the bin had exited 0 — the #89 class: runner-vs-sandbox divergence).
# A leg that loses its evidence line is a leg that cannot be audited:
# an empty collection dies loud, below.
echo "$OUT_LH" | grep -E "longhaul" >> "$EVID" || true
echo "$OUT_LH" | grep -E "^EVIDENCE" | tee -a "$EVID" >/dev/null || true
collect_last="$(echo "$OUT_LH" | grep -E "^EVIDENCE" | tail -1)"
[ -n "$collect_last" ] || die "the longhaul bin printed no EVIDENCE line — the leg's shape is unauditable"
say "$collect_last"

# --- server truth (read-back DIRECT — a paced GET would take hours) ---------
st="$(s3_status HEAD "$DIRECT_KEY")"
[ "$st" = "200" ] || die "object HEAD -> $st (the resumed leg must complete the upload)"
s3_get_to "$DIRECT_KEY" "$RUN/longhaul-received.bin" >/dev/null
[ "$(sha_of "$RUN/longhaul-received.bin")" = "$(sha_of "$FILE")" ] || \
  die "read-back sha256 mismatch (pause/resume across the journal must be byte-exact)"
echo "EVIDENCE {\"scenario\":\"longhaul-readback\",\"sha256_ok\":true,\"rate_Bps\":$RATE,\"timebox_s\":$TIMEBOX}" | tee -a "$EVID" >/dev/null
rm -f "$RUN/longhaul-received.bin"

say "GREEN — longhaul leg honest (journal replayable, state truthful, byte-exact)"
say "artifact: $EVID"
