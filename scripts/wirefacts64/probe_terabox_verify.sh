#!/usr/bin/env bash
# Wire-facts probe v4 for #64 — the VERIFY ROUTE on a multi-slice file.
# The multipart probe left "no dlink anywhere" — this pins WHICH download
# route works (filemetas dlink / PCS method=download / api/download),
# with raw responses, on a 9 MiB 3-slice upload. NEVER echoes cookies.
set -u
DATA=/tmp/wf64v; rm -rf $DATA; mkdir -p $DATA
JAR=/home/z/my-project/.secrets/terabox_cookies.txt
UA="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"

NDUS=$(awk -F'\t' '$6=="ndus"{print $7}' $JAR | head -1)
CK="lang=en; ndus=$NDUS"
BASE="https://dm.terabox.com"

curl -s -b "$CK" -A "$UA" -L "$BASE/main" -o $DATA/main.html
JT=$(python3 - << 'EOF'
import re, urllib.parse
h = open('/tmp/wf64v/main.html', encoding='utf-8', errors='ignore').read()
m = re.search(r'fn%28%22([A-Fa-f0-9]{32,})%22%29', h)
if not m:
    d = urllib.parse.unquote(h)
    m = re.search(r'fn\("([A-Fa-f0-9]{32,})"\)', d)
print(m.group(1) if m else '')
EOF
)
[ -z "$JT" ] && { echo "jsToken NOT found"; exit 1; }
LOGID=$(python3 -c "import secrets;print(secrets.token_hex(16))")
Q="app_id=250528&web=1&channel=dubox&clienttype=0&jsToken=$JT&dp-logid=$LOGID"

head -c 9437184 /dev/urandom > $DATA/e2e.bin
SIZE=$(stat -c%s $DATA/e2e.bin); BLK=4194304
MD5_0=$(head -c $BLK $DATA/e2e.bin | md5sum | cut -d' ' -f1)
MD5_1=$(tail -c +$((BLK+1)) $DATA/e2e.bin | head -c $BLK | md5sum | cut -d' ' -f1)
MD5_2=$(tail -c +$((2*BLK+1)) $DATA/e2e.bin | md5sum | cut -d' ' -f1)
BLOCKS="[\"$MD5_0\",\"$MD5_1\",\"$MD5_2\"]"
P="/apps/ium_drill/verify9m.bin"

R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/precreate?$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" --data-urlencode "autoinit=1" \
  --data-urlencode "target_path=/apps/ium_drill/" \
  --data-urlencode "block_list=$BLOCKS" --data-urlencode "size=$SIZE")
UPID=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('uploadid',''))")
[ -z "$UPID" ] && { echo "precreate failed: $R"; exit 1; }

UHOST=$(curl -s -b "$CK" -A "$UA" "$BASE/rest/2.0/pcs/file?method=locateupload&$Q" | \
  python3 -c "import json,sys;print((json.load(sys.stdin).get('server') or [''])[0])")
[ -z "$UHOST" ] && { echo "no upload host"; exit 1; }

for i in 0 1 2; do
  OFF=$((i*BLK)); LEN=$BLK; [ $i -eq 2 ] && LEN=$((SIZE-2*BLK))
  tail -c +$((OFF+1)) $DATA/e2e.bin | head -c $LEN > $DATA/part$i.bin
  curl -s -o /dev/null -w "part$i HTTP %{http_code}\n" -b "$CK" -A "$UA" -e "$BASE/main" \
    -X POST "https://$UHOST/rest/2.0/pcs/superfile2?method=upload&app_id=250528&channel=dubox&clienttype=0&web=1&path=$(python3 -c "import urllib.parse;print(urllib.parse.quote('$P'))")&uploadid=$UPID&uploadsign=0&partseq=$i" \
    -F "file=@$DATA/part$i.bin"
done
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/create?$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" --data-urlencode "size=$SIZE" --data-urlencode "uploadid=$UPID" \
  --data-urlencode "block_list=$BLOCKS" --data-urlencode "target_path=/apps/ium_drill/" \
  --data-urlencode "local_mtime=$(date +%s)" --data-urlencode "isdir=0" --data-urlencode "rtype=1")
echo "create errno: $(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('errno'))")"
PQ=$(python3 -c "import urllib.parse;print(urllib.parse.quote('$P'))")

echo "== Route A: filemetas dlink=1 =="
sleep 2
FSID=$(curl -s -b "$CK" -A "$UA" "$BASE/api/list?$Q&dir=%2Fapps%2Fium_drill&order=name&num=100&page=1&showempty=0" | \
  python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((str(i.get('fs_id')) for i in items if i.get('server_filename')=='verify9m.bin'),''))")
echo "fs_id=$FSID"
R=$(curl -s -b "$CK" -A "$UA" "$BASE/api/filemetas?$Q&fsids=%5B$FSID%5D&dlink=1")
echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
for i in items:
    dl=i.get('dlink','')
    print('filemetas keys:', sorted(i.keys())[:12])
    print('dlink present:', bool(dl), 'len', len(dl) if dl else 0)" 2>&1 | head -5
DLA=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((i.get('dlink','') for i in items),''))" 2>/dev/null)
if [ -n "$DLA" ]; then
  curl -s -b "$CK" -A "$UA" -L "$DLA" -o $DATA/backA.bin
  cmp -s $DATA/e2e.bin $DATA/backA.bin && echo "Route A roundtrip: BYTE-EXACT YES" || echo "Route A: bytes differ"
else
  echo "Route A: no dlink"
fi

echo "== Route B: PCS method=download (the W8 route) =="
CODE=$(curl -s -b "$CK" -A "$UA" -o $DATA/backB.bin -w "%{http_code}" -L \
  "$BASE/rest/2.0/pcs/file?method=download&app_id=250528&web=1&channel=dubox&clienttype=0&jsToken=$JT&path=$PQ")
cmp -s $DATA/e2e.bin $DATA/backB.bin && echo "Route B (HTTP $CODE) roundtrip: BYTE-EXACT YES" || \
  { echo "Route B HTTP $CODE not byte-exact; body head:"; head -c 200 $DATA/backB.bin; echo; }

echo "== Route C: /api/download =="
CODE=$(curl -s -b "$CK" -A "$UA" -o $DATA/backC.bin -w "%{http_code}" -L \
  "$BASE/api/download?$Q&path=$PQ")
cmp -s $DATA/e2e.bin $DATA/backC.bin && echo "Route C (HTTP $CODE) roundtrip: BYTE-EXACT YES" || \
  { echo "Route C HTTP $CODE not byte-exact; body head:"; head -c 200 $DATA/backC.bin; echo; }

echo "== cleanup =="
curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/filemanager?opera=delete&async=1&$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "filelist=[\"$P\"]" | head -c 120; echo
echo "DONE."
