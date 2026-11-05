#!/usr/bin/env bash
# Wire-facts rehearsal for #64 — the TeraBox community web API, LIVE, with the owner's cookies.
# Reads .secrets/terabox_cookies.txt (Netscape jar). NEVER echoes cookie values.
# Every stage records its verdict and CONTINUES — one run = maximum wire facts.
set -u
DATA=/tmp/wf64; rm -rf $DATA; mkdir -p $DATA
JAR=/home/z/my-project/.secrets/terabox_cookies.txt
UA="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
APP="app_id=250528&web=1&channel=dubox&clienttype=0"
# W0-wirefact (first run): www.terabox.com/api/* answers errno -6 'user not login' with the
# SAME jar; dm.terabox.com/api/* answers errno 0 — the LIVE web-API surface is dm.terabox.com.
BASE="https://dm.terabox.com"
HDR=(-b $JAR -A "$UA" -e "$BASE/main" -H "Origin: $BASE")

q() { python3 -c "import json,sys;d=json.load(sys.stdin);print(json.dumps(d)[:400])" 2>/dev/null || echo "(non-JSON or empty)"; }

echo "== W1: the credential verdict — /api/quota =="
R=$(curl -s "${HDR[@]}" "$BASE/api/quota?checkfree=1&checkexpire=1&$APP")
echo "$R" | q

echo "== W2: precreate (small file, one 4MiB-class block) =="
head -c 131072 /dev/urandom > $DATA/t.bin          # 128 KiB probe file
MD5=$(md5sum $DATA/t.bin | cut -d' ' -f1)
SIZE=$(stat -c%s $DATA/t.bin)
P="/apps/ium_drill/t.bin"
echo "local file: $SIZE bytes, md5 $MD5 (path $P)"
R=$(curl -s "${HDR[@]}" -X POST "$BASE/api/precreate?$APP" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" \
  --data-urlencode "size=$SIZE" \
  --data-urlencode "uploadid=" \
  --data-urlencode "block_list=[\"$MD5\"]" \
  --data-urlencode "target_path=/apps/ium_drill/")
echo "$R" | q
UPID=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('uploadid',''))" 2>/dev/null)
echo "uploadid: ${UPID:-NONE}"

if [ -n "$UPID" ]; then
  echo "== W3: superfile2 PUT (the byte block) =="
  # try the generic pcs host first; regional variants (c-jp / d3 / ... ) get pinned from precreate's host field if present
  HOST=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('host','d.terabox.com'))" 2>/dev/null)
  echo "host hint: ${HOST:-none-given}"
  for H in "$HOST" "d.terabox.com" "c-jp.terabox.com" "dm.terabox.com"; do
    [ -z "$H" ] && continue
    C=$(curl -s "${HDR[@]}" -o $DATA/sup.out -w "%{http_code}" -X PUT \
      "https://$H/rest/2.0/pcs/superfile2?method=upload&uploadid=$UPID&path=$P&partseq=0&$APP" \
      -H "Content-Type: application/octet-stream" -T $DATA/t.bin)
    echo "host $H -> HTTP $C, body: $(head -c 200 $DATA/sup.out)"
    [ "$C" = "200" ] && break
  done

  echo "== W4: create (finalize) =="
  R=$(curl -s "${HDR[@]}" -X POST "$BASE/api/create?$APP" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "path=$P" \
    --data-urlencode "size=$SIZE" \
    --data-urlencode "uploadid=$UPID" \
    --data-urlencode "block_list=[\"$MD5\"]" \
    --data-urlencode "target_path=/apps/ium_drill/" \
    --data-urlencode "local_mtime=$(date +%s)" \
    --data-urlencode "isdir=0")
  echo "$R" | sed -E 's/"(dlink|sign|download_ad"[^,]*)"/"\1-TRUNCATED"/g' | q
fi

echo "== W5: cleanup — delete the probe file =="
R=$(curl -s "${HDR[@]}" -X POST "$BASE/api/filemanager?opera=delete&async=1&$APP" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "filelist=[\"$P\"]")
echo "$R" | q
echo "DONE — every verdict above is a wire fact."
