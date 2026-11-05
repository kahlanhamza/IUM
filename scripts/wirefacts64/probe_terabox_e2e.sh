#!/usr/bin/env bash
# Wire-facts probe v2 for #64 — FULL write flow with the community-protocol corrections:
#   precreate(autoinit=1) -> uploadid -> superfile2 multipart-form(file, uploadsign=0, partseq=0)
#   -> create(rtype=1) -> list -> download byte-compare -> delete.
# Proven sources: Pahadi10/terabox-upload-tool lib/ (fetched 2026-11-06).
# Reads .secrets/terabox_cookies.txt. NEVER echoes cookie or token values.
set -u
DATA=/tmp/wf64b; rm -rf $DATA; mkdir -p $DATA
JAR=/home/z/my-project/.secrets/terabox_cookies.txt
UA="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"

NDUS=$(awk -F'\t' '$6=="ndus"{print $7}' $JAR | head -1)
[ -z "$NDUS" ] && { echo "no ndus in jar"; exit 1; }
CK="lang=en; ndus=$NDUS"
BASE="https://dm.terabox.com"

# jsToken self-serve: embedded in the logged-in main page (W-wirefact from run 1)
curl -s -b "$CK" -A "$UA" -L "$BASE/main" -o $DATA/main.html
JT=$(python3 - << 'EOF'
import re, urllib.parse
h = open('/tmp/wf64b/main.html', encoding='utf-8', errors='ignore').read()
m = re.search(r'fn%28%22([A-Fa-f0-9]{32,})%22%29', h)
if not m:
    d = urllib.parse.unquote(h)
    m = re.search(r'fn\("([A-Fa-f0-9]{32,})"\)', d)
print(m.group(1) if m else '')
EOF
)
[ -z "$JT" ] && { echo "jsToken NOT found on page — abort"; exit 1; }
echo "jsToken extracted: ${JT:0:6}...(len ${#JT})"
LOGID=$(python3 -c "import secrets;print(secrets.token_hex(16))")
Q="app_id=250528&web=1&channel=dubox&clienttype=0&jsToken=$JT&dp-logid=$LOGID"

head -c 2097152 /dev/urandom > $DATA/e2e.bin    # 2 MiB
MD5=$(md5sum $DATA/e2e.bin | cut -d' ' -f1)
SIZE=$(stat -c%s $DATA/e2e.bin)
P="/apps/ium_drill/e2e.bin"
echo "probe file: $SIZE bytes, md5 ${MD5:0:8}..., path $P"

echo "== P1: precreate (autoinit=1) =="
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/precreate?$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" --data-urlencode "autoinit=1" \
  --data-urlencode "target_path=/apps/ium_drill/" \
  --data-urlencode "block_list=[\"$MD5\"]" --data-urlencode "size=$SIZE")
echo "$R" | head -c 300; echo
UPID=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('uploadid',''))" 2>/dev/null)
RTYPE=$(echo "$R" | python3 -c "import json,sys;print(json.load(sys.stdin).get('return_type',''))" 2>/dev/null)
echo "uploadid: ${UPID:+PRESENT(${#UPID} chars)}${UPID:-NONE}  return_type: $RTYPE"
[ -z "$UPID" ] && { echo "no uploadid — flow stops here (wire fact recorded)"; exit 0; }

echo "== P2: locateupload — the per-session upload host assignment =="
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" "https://dm.terabox.com/rest/2.0/pcs/file?method=locateupload&$Q")
echo "$R" | head -c 300; echo
UHOST=$(echo "$R" | python3 -c "import json,sys;d=json.load(sys.stdin);print((d.get('server') or [''])[0])" 2>/dev/null)
echo "assigned upload host: ${UHOST:-NONE}"
[ -z "$UHOST" ] && { echo "no host assigned — wire fact recorded"; exit 0; }

echo "== P3: superfile2 multipart-form PUT on $UHOST =="
C=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -o $DATA/sup.out -w "%{http_code}" \
  -X POST "https://$UHOST/rest/2.0/pcs/superfile2?method=upload&app_id=250528&channel=dubox&clienttype=0&web=1&path=$(python3 -c "import urllib.parse;print(urllib.parse.quote('$P'))")&uploadid=$UPID&uploadsign=0&partseq=0" \
  -F "file=@$DATA/e2e.bin")
echo "$UHOST -> HTTP $C, body: $(head -c 200 $DATA/sup.out)"

echo "== P4: create (rtype=1) =="
R=$(curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/create?$Q" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "path=$P" --data-urlencode "size=$SIZE" \
  --data-urlencode "uploadid=$UPID" \
  --data-urlencode "block_list=[\"$MD5\"]" \
  --data-urlencode "target_path=/apps/ium_drill/" \
  --data-urlencode "local_mtime=$(date +%s)" --data-urlencode "isdir=0" --data-urlencode "rtype=1")
echo "$R" | sed -E 's/("(dlink|sign|downloadAd)[^"]*"):"[^"]{20,}"/\1:"TRUNCATED"/g' | head -c 400; echo

echo "== P5: list — is the file there? =="
R=$(curl -s -b "$CK" -A "$UA" "$BASE/api/list?$Q&dir=%2Fapps%2Fium_drill&order=name&num=100&page=1&showempty=0")
echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print('errno', d.get('errno'), '- files:', [(i.get('server_filename'), i.get('size')) for i in items])"
DL=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((i.get('dlink','') for i in items if i.get('server_filename')=='e2e.bin'),''))" 2>/dev/null)

if [ -z "$DL" ]; then
  echo "== P5b: filemetas (fsids + dlink=1) — the proper dlink source =="
  FSID=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((str(i.get('fs_id')) for i in items if i.get('server_filename')=='e2e.bin'),''))" 2>/dev/null)
  if [ -n "$FSID" ]; then
    R=$(curl -s -b "$CK" -A "$UA" "$BASE/api/filemetas?$Q&fsids=%5B$FSID%5D&dlink=1")
    echo "$R" | sed -E 's/("dlink":"[^"]{40})[^"]*/\1...TRUNCATED/' | head -c 400; echo
    DL=$(echo "$R" | python3 -c "
import json,sys
d=json.load(sys.stdin)
items=d.get('list') or []
print(next((i.get('dlink','') for i in items),''))" 2>/dev/null)
  fi
fi

echo "== P6: download back + byte-compare =="
if [ -n "$DL" ]; then
  curl -s -b "$CK" -A "$UA" -L "$DL" -o $DATA/back.bin
  cmp -s $DATA/e2e.bin $DATA/back.bin && echo "2MiB roundtrip via dlink: BYTE-EXACT YES" || echo "roundtrip byte-exact: NO"
else
  echo "no dlink in list response (wire fact); trying /api/download"
  curl -s -b "$CK" -A "$UA" -L "$BASE/api/download?$Q&path=$(python3 -c "import urllib.parse;print(urllib.parse.quote('$P'))")" -o $DATA/back2.bin -w "HTTP %{http_code}\n"
  cmp -s $DATA/e2e.bin $DATA/back2.bin && echo "roundtrip via /api/download: BYTE-EXACT YES" || echo "roundtrip byte-exact: NO"
fi

if [ "${SKIP_DELETE:-0}" = "1" ]; then
  echo "== P7: cleanup SKIPPED (SKIP_DELETE=1 — file left on the account) =="
else
  echo "== P7: cleanup =="
  curl -s -b "$CK" -A "$UA" -e "$BASE/main" -X POST "$BASE/api/filemanager?opera=delete&async=1&$Q" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "filelist=[\"$P\"]" | head -c 150; echo
fi
echo "DONE."
