#!/usr/bin/env bash
# Wire-facts probe for the rclone-bridge lane (issue #63 candidate).
# Question: can IUM's EXISTING S3 adapter (adapter_s3 + sigv4) drive
# `rclone serve s3` against a :memory: remote — single PUT, GET-back verify,
# and above all MULTIPART upload (the chunk pool's shape)?
set -u
export PATH=/home/z/my-project/.toolchain/bin:$PATH
R=/tmp/serve63; rm -rf $R; mkdir -p $R
PORT=5533; AK=iumak; SK=iumsk
DATA=/tmp/wf63; rm -rf $DATA; mkdir -p $DATA
head -c 3000000 /dev/urandom > $DATA/big.bin   # 3 MiB
head -c 9000000  /dev/urandom > $DATA/huge.bin # 9 MiB

rclone serve s3 /tmp/s3root --addr 127.0.0.1:$PORT > $R/serve.log 2>&1 &
SRV=$!
sleep 2

SIG="aws:amz:us-east-1:s3"; AUTH="-u $AK:$SK"
B=http://127.0.0.1:$PORT

echo "== W1: single PUT (3 MiB) =="
curl -s -o /dev/null -w "%{http_code}\n" --aws-sigv4 $SIG $AUTH -X PUT -T $DATA/big.bin $B/ium/test.bin

echo "== W2: HEAD =="
curl -s --aws-sigv4 $SIG $AUTH -I $B/ium/test.bin | grep -iE "^(HTTP|content-length|etag)" | tr -d '\r'

echo "== W3: GET back + byte-compare =="
curl -s --aws-sigv4 $SIG $AUTH $B/ium/test.bin -o $DATA/back.bin && cmp -s $DATA/big.bin $DATA/back.bin && echo "3MiB byte-exact: YES" || echo "3MiB byte-exact: NO"

echo "== W4: multipart init (POST ?uploads) =="
U=$(curl -s --aws-sigv4 $SIG $AUTH -X POST "$B/ium/mp.bin?uploads" | grep -o "<UploadId>[^<]*" | cut -d'>' -f2)
echo "UploadId: ${U:-NONE}"

if [ -n "${U:-}" ]; then
  echo "== W5: upload 2 parts =="
  head -c 5242880 $DATA/huge.bin > $DATA/p1; tail -c +5242881 $DATA/huge.bin > $DATA/p2
  P1=$(curl -s -o /dev/null -w "%{http_code}" --aws-sigv4 $SIG $AUTH -X PUT "$B/ium/mp.bin?partNumber=1&uploadId=$U" -T $DATA/p1)
  P2=$(curl -s -o /dev/null -w "%{http_code}" --aws-sigv4 $SIG $AUTH -X PUT "$B/ium/mp.bin?partNumber=2&uploadId=$U" -T $DATA/p2)
  echo "part1: $P1 part2: $P2"
  echo "== W6: complete multipart =="
  COMP="<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"n1\"</ETag></Part><Part><PartNumber>2</PartNumber><ETag>\"n2\"</ETag></Part></CompleteMultipartUpload>"
  curl -s --aws-sigv4 $SIG $AUTH -X POST "$B/ium/mp.bin?uploadId=$U" -H 'Content-Type: application/xml' -d "$COMP" | head -c 400; echo
  echo "== W7: GET the multipart object + byte-compare =="
  curl -s --aws-sigv4 $SIG $AUTH $B/ium/mp.bin -o $DATA/mpback.bin && cmp -s $DATA/huge.bin $DATA/mpback.bin && echo "9MiB multipart byte-exact: YES" || echo "9MiB multipart byte-exact: NO"
else
  echo "== multipart NOT offered — pin and move on =="
fi

kill $SRV 2>/dev/null; wait $SRV 2>/dev/null
echo "serve.log tail:"; tail -5 $R/serve.log
