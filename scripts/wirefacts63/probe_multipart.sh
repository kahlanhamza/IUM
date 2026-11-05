#!/usr/bin/env bash
# Wire-facts probe #2 for the rclone-bridge lane: MULTIPART against
# `rclone serve s3` (anonymous, local fs root — the mode proven 200 in probe #1).
# Decides: can IUM's chunk pool (multipart shape) ride the bridge?
set -u
export PATH=/home/z/my-project/.toolchain/bin:$PATH
PORT=5543
DATA=/tmp/wf63b; rm -rf $DATA; mkdir -p $DATA /tmp/s3root/bkt
head -c 9000000 /dev/urandom > $DATA/huge.bin          # 9 MiB
head -c 5242880 $DATA/huge.bin > $DATA/p1              # part 1: 5 MiB (min part size)
tail -c +5242881 $DATA/huge.bin > $DATA/p2             # part 2: 4 MiB

rclone serve s3 /tmp/s3root --addr 127.0.0.1:$PORT > $DATA/serve.log 2>&1 &
SRV=$!
sleep 2
B=http://127.0.0.1:$PORT

echo "== M1: multipart init =="
INIT=$(curl -s -X POST "$B/bkt/mp.bin?uploads")
U=$(echo "$INIT" | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
echo "UploadId: ${U:-NONE}  (init response: $(echo "$INIT" | head -c 200))"

echo "== M2: two parts (capture real ETags) =="
C1=$(curl -s -D $DATA/h1 -o /dev/null -w "%{http_code}" -X PUT "$B/bkt/mp.bin?partNumber=1&uploadId=$U" -T $DATA/p1 -H 'Content-Type: application/octet-stream')
C2=$(curl -s -D $DATA/h2 -o /dev/null -w "%{http_code}" -X PUT "$B/bkt/mp.bin?partNumber=2&uploadId=$U" -T $DATA/p2 -H 'Content-Type: application/octet-stream')
E1=$(grep -i '^etag:' $DATA/h1 | tr -d '\r' | cut -d' ' -f2)
E2=$(grep -i '^etag:' $DATA/h2 | tr -d '\r' | cut -d' ' -f2)
echo "part1: $C1 etag1: $E1  part2: $C2 etag2: $E2"

echo "== M3: complete (real ETags) =="
COMP="<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>$E1</ETag></Part><Part><PartNumber>2</PartNumber><ETag>$E2</ETag></Part></CompleteMultipartUpload>"
curl -s -X POST "$B/bkt/mp.bin?uploadId=$U" -H 'Content-Type: application/xml' -d "$COMP" | head -c 300; echo

echo "== M4: GET back + byte-compare =="
curl -s "$B/bkt/mp.bin" -o $DATA/mpback.bin
cmp -s $DATA/huge.bin $DATA/mpback.bin && echo "9MiB via 2-part multipart byte-exact: YES" || echo "9MiB via 2-part multipart byte-exact: NO"
echo "on disk:"; ls -la /tmp/s3root/bkt/mp.bin 2>/dev/null

kill $SRV 2>/dev/null; wait $SRV 2>/dev/null
echo "serve.log tail:"; tail -4 $DATA/serve.log
