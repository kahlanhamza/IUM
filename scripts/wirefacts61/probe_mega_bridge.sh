#!/usr/bin/env bash
# Wire-facts rehearsal for #61 — MEGA through the #63 rclone bridge, LIVE, real credentials.
# Reads .secrets/mega.json. NEVER echoes any secret value (mask everything).
set -u
export PATH=/home/z/my-project/.toolchain/bin:$PATH
DATA=/tmp/wf61; rm -rf $DATA; mkdir -p $DATA

SECRETS=/home/z/my-project/.secrets/mega.json
MUSER=$(python3 -c "import json;print(json.load(open('$SECRETS'))['user'])")
MPASS=$(python3 -c "import json;print(json.load(open('$SECRETS'))['pass'])")
export RCLONE_CONFIG_MEGA_TYPE=mega
export RCLONE_CONFIG_MEGA_USER="$MUSER"
export RCLONE_CONFIG_MEGA_PASS="$(rclone obscure "$MPASS")"
unset MPASS

echo "== W1: login + root listing (the credential verdict) =="
if rclone lsd mega: > $DATA/lsd.out 2> $DATA/lsd.err; then
  echo "login OK; root entries: $(wc -l < $DATA/lsd.out)"
else
  echo "login FAILED:"; sed -E 's/(user|pass|token)[^ ]*/MASKED/gi' $DATA/lsd.err | tail -3; exit 1
fi

echo "== W2: rclone-wire roundtrip on the real backend =="
head -c 1048576 /dev/urandom > $DATA/one.bin
rclone mkdir mega:ium-drill && echo "mkdir ium-drill OK"
rclone copyto $DATA/one.bin mega:ium-drill/one.bin && echo "copyto OK"
rclone cat mega:ium-drill/one.bin > $DATA/one.back 2>/dev/null
cmp -s $DATA/one.bin $DATA/one.back && echo "1MiB rclone roundtrip byte-exact: YES" || echo "1MiB rclone roundtrip byte-exact: NO"

echo "== W3: serve s3 over the mega ROOT (bucket-less backend topology) =="
# W3-wirefact: serve s3 parses the FIRST path segment as the bucket; a bucket-less
# backend must be served from its ROOT with a top-level folder as the bucket
# (first attempt serving mega:ium-drill directly mis-parsed '/mp.bin' as bucket 'mp.bin').
PORT=5611
rclone serve s3 mega: --addr 127.0.0.1:$PORT > $DATA/serve.log 2>&1 &
SRV=$!
sleep 4
B=http://127.0.0.1:$PORT
C=$(curl -s -o /dev/null -w "%{http_code}" -X PUT "$B/ium-drill/one2.bin" -T $DATA/one.bin)
curl -s "$B/ium-drill/one2.bin" -o $DATA/one2.back
cmp -s $DATA/one.bin $DATA/one2.back && X=YES || X=NO
echo "sigv4-wire single PUT: $C, byte-exact GET-back: $X"

echo "== W4-W7: multipart over the LIVE mega backend =="
head -c 9000000 /dev/urandom > $DATA/mp.bin
head -c 5242880 $DATA/mp.bin > $DATA/p1
tail -c +5242881 $DATA/mp.bin > $DATA/p2
INIT=$(curl -s -X POST "$B/ium-drill/mp.bin?uploads")
U=$(echo "$INIT" | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
echo "UploadId present: $([ -n "$U" ] && echo YES || echo NO)"
C1=$(curl -s -D $DATA/h1 -o /dev/null -w "%{http_code}" -X PUT "$B/ium-drill/mp.bin?partNumber=1&uploadId=$U" -T $DATA/p1 -H 'Content-Type: application/octet-stream')
C2=$(curl -s -D $DATA/h2 -o /dev/null -w "%{http_code}" -X PUT "$B/ium-drill/mp.bin?partNumber=2&uploadId=$U" -T $DATA/p2 -H 'Content-Type: application/octet-stream')
E1=$(grep -i '^etag:' $DATA/h1 | tr -d '\r' | cut -d' ' -f2)
E2=$(grep -i '^etag:' $DATA/h2 | tr -d '\r' | cut -d' ' -f2)
echo "part1: $C1  part2: $C2  etags present: $([ -n "$E1" ] && echo YES)/$([ -n "$E2" ] && echo YES)"
COMP="<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>$E1</ETag></Part><Part><PartNumber>2</PartNumber><ETag>$E2</ETag></Part></CompleteMultipartUpload>"
echo "complete: $(curl -s -X POST "$B/ium-drill/mp.bin?uploadId=$U" -H 'Content-Type: application/xml' -d "$COMP" | head -c 200)"
curl -s "$B/ium-drill/mp.bin" -o $DATA/mp.back
cmp -s $DATA/mp.bin $DATA/mp.back && echo "9MiB 2-part multipart via LIVE mega: byte-exact YES" || echo "9MiB multipart byte-exact: NO"

kill $SRV 2>/dev/null; wait $SRV 2>/dev/null
echo "== cleanup =="
if rclone purge mega:ium-drill 2>/dev/null; then echo "remote dir ium-drill purged"; else echo "purge failed (harmless; remote dir remains)"; fi
echo "serve.log tail:"; sed -E 's/(user|pass|token)[^ ]*/MASKED/gi' $DATA/serve.log | tail -3
