#!/usr/bin/env bash
# wirefacts57 — issue #57 (M9#2): the S3-shaped link prober's wire facts,
# rehearsed LIVE before the design note claims anything.
#
# The question the prober must answer before it exists: can a multipart
# PROBE be cheap? The probe shape is create -> small part PUTs (the
# canary bytes) -> ABORT (never complete — completing a probe would
# materialize junk and the 5 MiB floor would forbid small parts anyway).
#
# Vectors (pinned servers — rclone serve s3 and SeaweedFS 4.51):
#   WF0  readiness/bucket facts (anonymous GET works; service-level signed
#        ops on gofakes3 break mid-transfer — the prober speaks OBJECT ops);
#   WF0c the rehearsal's own curl lesson: --aws-sigv4 takes ONE argument
#        (provider:region:service), keys ride --user — a second "aws:iam::"
#        string is parsed as a SECOND URL (curl error 3) and the request
#        silently goes out anonymous;
#   WF1  UploadPart accepts a 64 KiB part into a LIVE multipart upload
#        (the 5 MiB floor binds at Complete, not at UploadPart — pin it);
#   WF2  4 concurrent part PUTs on ONE upload all succeed (parts are
#        independent resources — the K-phase shape);
#   WF3  ListParts mid-flight lists the parts (the reconcile shape);
#   WF4  CompleteMultipartUpload with all-small parts is REJECTED (the
#        floor is real — the probe must abort, never complete);
#   WF5  AbortMultipartUpload -> 204, and ListParts after abort -> 404
#        NoSuchUpload (the probe leaves NO orphan);
#   WF6  wall cost of a full create -> 1x64KiB -> abort transaction
#        (the probe's per-transaction budget, localhost).
#
# Usage: probe_floor_abort.sh rclone <root-dir>   (anonymous serve, W8/W12)
#        probe_floor_abort.sh seaweedfs <weed-binary> <work-dir>
set -euo pipefail

SERVER="${1:-}"
SIG="aws:amz:us-east-1:s3"
DATA=$(mktemp -d)

# 64 KiB canary — the prober's CANARY_MIN.
dd if=/dev/urandom of="$DATA/c.bin" bs=1024 count=64 status=none

case "$SERVER" in
  rclone)
    ROOT="${2:?root dir required}"
    mkdir -p "$ROOT/probe57"
    BIN="${RCLONE_BIN:-$PWD/.toolchain/bin/rclone}"
    [ -x "$BIN" ] || BIN="rclone"
    PORT=$(( (RANDOM % 20000) + 20000 ))
    "$BIN" serve s3 "$ROOT" --addr "127.0.0.1:$PORT" > "$DATA/serve.log" 2>&1 &
    SRV=$!
    B="http://127.0.0.1:$PORT"
    # W12 shape: well-formed SigV4 from throwaway keys against the
    # anonymous serve — the same credentials the bridge adapter signs with.
    CURL=(curl -s --aws-sigv4 "$SIG" --user "ium-bridge:ium-bridge-throwaway")
    ;;
  seaweedfs)
    WEED="${2:?weed binary required}"
    WORK="${3:?work dir required}"
    mkdir -p "$WORK/data"
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
    BASE=$(( (RANDOM % 20000) + 20000 ))
    "$WEED" server -dir="$WORK/data" -ip=127.0.0.1 \
      -master.port=$((BASE+1)) -volume.port=$((BASE+2)) \
      -s3 -s3.port="$BASE" -s3.config="$WORK/s3.json" -idleTimeout=0 \
      > "$DATA/weed.log" 2>&1 &
    SRV=$!
    B="http://127.0.0.1:$BASE"
    CURL=(curl -s --aws-sigv4 "$SIG" --user "iumcikey:iumcisecret00000000000000000000000000000000")
    ;;
  *) echo "usage: $0 rclone <root> | seaweedfs <weed> <work>" >&2; exit 2 ;;
esac
kill_server() { kill "$SRV" 2>/dev/null || true; wait "$SRV" 2>/dev/null || true; }
cleanup() { kill_server; rm -rf "$DATA"; }
trap cleanup EXIT

# WF0: readiness — anonymous GET (the anonymous serve answers `GET /` 200
# without sigv4; readiness must not depend on a shape the lane never rides).
ready=0
for _ in $(seq 1 120); do
  code=$(curl -s -o /dev/null --max-time 2 -w "%{http_code}" "$B/" || true)
  case "$code" in 200|301|403|404) ready=1; break ;; esac
  kill -0 "$SRV" 2>/dev/null || break
  sleep 0.5
done
[ "$ready" = 1 ] || { echo "server never ready"; tail -5 "$DATA"/*.log; exit 1; }
echo "WF0: server ready on $B (anonymous GET)"

# bucket — for the rclone leg the bucket IS a subdirectory of the serve
# root (W10, #63), pre-created above. A signed PutBucket is SKIPPED (WF0b,
# probed live: the anonymous gofakes3 serve breaks SIGNED service-level ops
# — ListBuckets on `/`, PutBucket on `/bkt` — mid-transfer, while signed
# OBJECT ops ride clean; the prober and the engine speak object ops only,
# and #63's bridge creates buckets via `rclone mkdir`, never the wire).

echo "=== WF1: 64 KiB UploadPart into a live multipart ==="
U=$("${CURL[@]}" -X POST "$B/probe57/probe.bin?uploads" \
      | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
[ -n "$U" ] || { echo "FAIL: no UploadId"; exit 1; }
echo "uploadId: ${U:0:24}..."
C1=$("${CURL[@]}" -o /dev/null -D "$DATA/h1" -w "%{http_code}" \
      -X PUT "$B/probe57/probe.bin?partNumber=1&uploadId=$U" -T "$DATA/c.bin")
E1=$(grep -i '^etag:' "$DATA/h1" | tr -d '\r' | cut -d' ' -f2)
echo "part1(64KiB) -> $C1 etag=${E1:0:20}..."
[ "$C1" = "200" ] && echo "WF1: PASS (small part accepted)" || { echo "WF1: FAIL"; exit 1; }

echo "=== WF2: 4 concurrent part PUTs on the same upload ==="
pids=(); codes=()
for pn in 2 3 4 5; do
  "${CURL[@]}" -o /dev/null -D "$DATA/h$pn" -w "%{http_code}" \
    -X PUT "$B/probe57/probe.bin?partNumber=$pn&uploadId=$U" -T "$DATA/c.bin" > "$DATA/code$pn" &
  pids+=($!)
done
for p in "${pids[@]}"; do wait "$p"; done
all200=1
for pn in 2 3 4 5; do
  c=$(cat "$DATA/code$pn"); codes+=("$c")
  [ "$c" = "200" ] || all200=0
done
echo "codes: ${codes[*]}"
[ "$all200" = 1 ] && echo "WF2: PASS (K=4 concurrent parts OK)" || { echo "WF2: FAIL"; exit 1; }

echo "=== WF3: ListParts mid-flight ==="
LP=$("${CURL[@]}" "$B/probe57/probe.bin?uploadId=$U")
N=$(echo "$LP" | grep -o "<Part>" | wc -l)
echo "listed parts: $N"
[ "$N" = "5" ] && echo "WF3: PASS" || { echo "WF3: FAIL (got $N)"; exit 1; }

echo "=== WF4: Complete with all-small parts (REAL etags) is REJECTED ==="
U4=$("${CURL[@]}" -X POST "$B/probe57/floor.bin?uploads" | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
[ -n "$U4" ] || { echo "WF4: FAIL (no upload)"; exit 1; }
COMP="<CompleteMultipartUpload>"
for pn in 1 2 3 4 5; do
  "${CURL[@]}" -o /dev/null -D "$DATA/fh$pn" -w "%{http_code}" \
    -X PUT "$B/probe57/floor.bin?partNumber=$pn&uploadId=$U4" -T "$DATA/c.bin" > /dev/null
  E=$(grep -i '^etag:' "$DATA/fh$pn" | tr -d '\r' | cut -d' ' -f2)
  [ -n "$E" ] || { echo "WF4: FAIL (no etag for part $pn)"; exit 1; }
  COMP="$COMP<Part><PartNumber>$pn</PartNumber><ETag>$E</ETag></Part>"
done
COMP="$COMP</CompleteMultipartUpload>"
C4=$("${CURL[@]}" -o "$DATA/comp.xml" -w "%{http_code}" \
      -X POST "$B/probe57/floor.bin?uploadId=$U4" -H "Content-Type: application/xml" -d "$COMP")
echo "complete(5 small parts, real etags) -> $C4  body: $(head -c 200 "$DATA/comp.xml")"
case "$C4" in
  200) echo "WF4: NOTE — server ACCEPTED small parts (non-standard; probe must still abort)" ;;
  *) echo "WF4: PASS (the floor/size gate binds at complete -> $C4)" ;;
esac

echo "=== WF5: Abort (fresh upload, never completed) -> 204, then ListParts -> 404 ==="
U5=$("${CURL[@]}" -X POST "$B/probe57/abort.bin?uploads" | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
[ -n "$U5" ] || { echo "WF5: FAIL (no upload)"; exit 1; }
"${CURL[@]}" -o /dev/null -X PUT "$B/probe57/abort.bin?partNumber=1&uploadId=$U5" -T "$DATA/c.bin"
C5=$("${CURL[@]}" -o /dev/null -w "%{http_code}" \
      -X DELETE "$B/probe57/abort.bin?uploadId=$U5")
echo "abort -> $C5"
L2=$("${CURL[@]}" -o /dev/null -w "%{http_code}" \
      "$B/probe57/abort.bin?uploadId=$U5")
echo "list-parts-after-abort -> $L2"
[ "$C5" = "204" ] && [ "$L2" = "404" ] && echo "WF5: PASS (no orphan)" \
  || echo "WF5: CHECK (abort=$C5 list-after=$L2)"

echo "=== WF6: full probe-transaction cost (create -> 1x64KiB -> abort), x5 ==="
total=0
for i in 1 2 3 4 5; do
  t0=$(date +%s%N)
  Ui=$("${CURL[@]}" -X POST "$B/probe57/p$i.bin?uploads" \
        | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
  "${CURL[@]}" -o /dev/null \
    -X PUT "$B/probe57/p$i.bin?partNumber=1&uploadId=$Ui" -T "$DATA/c.bin"
  "${CURL[@]}" -o /dev/null \
    -X DELETE "$B/probe57/p$i.bin?uploadId=$Ui"
  t1=$(date +%s%N)
  ms=$(( (t1 - t0) / 1000000 )); total=$((total + ms))
  echo "  tx$i: ${ms} ms"
done
echo "mean: $((total / 5)) ms"

kill_server
echo "=== wirefacts57 $SERVER leg DONE ==="
