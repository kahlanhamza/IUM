#!/usr/bin/env bash
# Wire-facts probe v3 for #64 — the MULTI-SLICE question (the adapter's geometry):
#   9 MiB as 3 blocks (4+4+1), block_list = [md5_0,md5_1,md5_2], partseq 0..2.
#   Also probed: OUT-OF-ORDER partseq (1 before 0) and a RE-PUT of a done partseq
#   (the resume-idempotency fact). Follows probe_terabox_e2e.sh's proven shapes.
# Reads .secrets/terabox_cookies.txt. NEVER echoes cookie or token values.
set -u
DATA=/tmp/wf64m; rm -rf $DATA; mkdir -p $DATA
JAR=/home/z/my-project/.secrets/terabox_cookies.txt
UA="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"

NDUS=$(awk -F'\t' '$6=="ndus"{print $7}' $JAR | head -1)
[ -z "$NDUS" ] && { echo "no ndus in jar"; exit 1; }
CK="lang=en; ndus=$NDUS"
BASE="https://dm.terabox.com"

curl -s -b "$CK" -A "$UA" -L "$BASE/main" -o $DATA/main.html
JT=$(python3 - << 'EOF'
import re, urllib.parse
h = open('/tmp/wf64m/main.html', encoding='utf-8', errors='ignore').read()
m = re.search(r'fn%28%22([A-Fa-f0-9]{32,})%22%29', h)
if not m:
    d = urllib.parse.unquote(h)
    m = re.search(r'fn\("([A-Fa-f0-9]{32,})"\)', d)
print(m.group(1) if m else '')
EOF
)
[ -z "$JT" ] && { echo "jsToken NOT found — cookies may have expired"; exit 1; }
echo "jsToken ok (${#JT} chars)"
LOGID=$(python3 -c "import secrets;print(secrets.token_hex(16))")
Q="app_id=250528&web=1&channel=dubox&clienttype=0&jsToken=$JT&dp-logid=$LOGID"

# 9 MiB = 3 blocks: 4 MiB + 4 MiB + 1 MiB (the Feishu-drill shape)
head -c 9437184 /dev/urandom > $DATA/e2e.bin
SIZE=$(stat -c%s $DATA/e2e.bin)
BLK=4194304
MD5_0=$(head -c $BLK $DATA/e2e.bin | md5sum | cut -d' ' -f1)
MD5_1=$(tail -c +$((BLK+1)) $DATA/e2e.bin | head -c $BLK | md5sum | cut -d' ' -f1)
MD5_2=$(tail -c +$((2*BLK+1)) $DATA/e2e.bin | md5sum | cut -d' ' -f1)
BLOCKS="[\"$MD5_0\",\"$MD5_1\",\"$MD5_2\"]"
P="/apps/ium_drill/multipart9m.bin"
echo "file: $SIZE bytes = 4M+4M+1M, path $P"

echo "== P1: precreate (autoinit=1, 3-entry block_list) =="
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/precreate?$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" --data-urlencode "autoinit=1" \
  --data-urlencode "target_path=/apps/ium_drill/" \
  --data-urlencode "block_list=$BLOCKS" --data-urlencode "size=$SIZE")
echo "$R" | head -c 300; echo
UPID=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('uploadid',''))" 2>/dev/null)
ERRNO=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('errno','?'))" 2>/dev/null)
echo "errno=$ERRNO uploadid: ${UPID:+PRESENT(${#UPID} chars)}${UPID:-NONE}"
[ -z "$UPID" ] && { echo "MULTI-BLOCK PRECREATE REFUSED — geometry verdict pinned"; exit 0; }

echo "== P2: locateupload =="
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" "$BASE/rest/2.0/pcs/file?method=locateupload&$Q")
UHOST=$(echo "$R" | python3 -c "import json,sys;d=json.load(sys.stdin);print((d.get('server') or [''])[0])" 2>/dev/null)
echo "assigned host: ${UHOST:-NONE}"
[ -z "$UHOST" ] && exit 0

put_part() {  # partseq, file-offset, length
  local SEQ=$1 OFF=$2 LEN=$3
  tail -c +$((OFF+1)) $DATA/e2e.bin | head -c $LEN > $DATA/part$SEQ.bin
  curl -s -b "$CK" -A "$UA" -e "$BASE/main" -o $DATA/sup$SEQ.out -w "%{http_code}" \
    -X POST "https://$UHOST/rest/2.0/pcs/superfile2?method=upload&app_id=250528&channel=dubox&clienttype=0&web=1&path=$(python3 -c "import urllib.parse;print(urllib.parse.quote('$P'))")&uploadid=$UPID&uploadsign=0&partseq=$SEQ" \
    -F "file=@$DATA/part$SEQ.bin"
}

echo "== P3: partseq 1 FIRST (out-of-order fact) =="
C=$(put_part 1 $BLK $BLK)
MD5W=$(md5sum $DATA/part1.bin | cut -d' ' -f1)
MD5G=$(python3 -c "import json;print(json.load(open('$DATA/sup1.out')).get('md5',''))" 2>/dev/null)
echo "HTTP $C, echoed md5 ${MD5G:0:8}... want ${MD5W:0:8}... match=$([ "$MD5W" = "$MD5G" ] && echo YES || echo NO)"

echo "== P4a: partseq 1 AGAIN — the TRUE re-PUT idempotency fact =="
C=$(put_part 1 $BLK $BLK)
MD5W=$(md5sum $DATA/part1.bin | cut -d' ' -f1)
MD5G=$(python3 -c "import json;print(json.load(open('$DATA/sup1.out')).get('md5',''))" 2>/dev/null)
echo "re-PUT part1 HTTP $C, md5 match=$([ \"$MD5W\" = \"$MD5G\" ] && echo YES || echo NO)"

echo "== P4b: partseq 0 then partseq 2 (tail 1 MiB) =="
C=$(put_part 0 0 $BLK)
echo "part0 HTTP $C: $(head -c 120 $DATA/sup0.out)"
C=$(put_part 2 $((2*BLK)) $((SIZE-2*BLK)))
echo "part2 HTTP $C: $(head -c 120 $DATA/sup2.out)"

echo "== P5: create (rtype=1) =="
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/create?$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" --data-urlencode "size=$SIZE" \
  --data-urlencode "uploadid=$UPID" --data-urlencode "block_list=$BLOCKS" \
  --data-urlencode "target_path=/apps/ium_drill/" \
  --data-urlencode "local_mtime=$(date +%s)" --data-urlencode "isdir=0" --data-urlencode "rtype=1")
echo "$R" | head -c 300; echo
FSID=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('fs_id',''))" 2>/dev/null)
echo "fs_id: ${FSID:-NONE}"

echo "== P6: list (+filemetas fallback) + download + byte-compare =="
sleep 2   # the async-index lag the first multipart run hit (no dlink at t=0)
R=$(curl -s -b "$CK" -A "$UA" "$BASE/api/list?$Q&dir=%2Fapps%2Fium_drill&order=name&num=100&page=1&showempty=0")
FSID=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((str(i.get('fs_id')) for i in items if i.get('server_filename')=='multipart9m.bin'),''))" 2>/dev/null)
DL=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((i.get('dlink','') for i in items if i.get('server_filename')=='multipart9m.bin'),''))" 2>/dev/null)
if [ -z "$DL" ] && [ -n "$FSID" ]; then
  echo "list without dlink; trying filemetas fs_id=$FSID"
  R=$(curl -s -b "$CK" -A "$UA" "$BASE/api/filemetas?$Q&fsids=%5B$FSID%5D&dlink=1")
  DL=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((i.get('dlink','') for i in items),''))" 2>/dev/null)
fi
if [ -n "$DL" ]; then
  curl -s -b "$CK" -A "$UA" -L "$DL" -o $DATA/back.bin
  cmp -s $DATA/e2e.bin $DATA/back.bin && echo "9MiB 3-slice roundtrip: BYTE-EXACT YES" || echo "roundtrip byte-exact: NO"
else
  echo "no dlink anywhere (wire fact — verify path needs its own route)"
fi

echo "== P7: cleanup =="
curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/filemanager?opera=delete&async=1&$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "filelist=[\"$P\"]" | head -c 150; echo
echo "DONE."
