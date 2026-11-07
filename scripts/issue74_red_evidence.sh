#!/usr/bin/env bash
# issue74 RED evidence — the class exposure, demonstrated on the real tree.
#
# Without .gitattributes, `git clone -c core.autocrlf=true` (what a Windows
# contributor's default is) produces CRLF on disk for every text file. This
# script counts the byte-diverging files at HEAD and demonstrates the exact
# lane-6 failure class: the brand sheet's $-anchored table regex parses
# ZERO rows from the CRLF copy.
set -euo pipefail
cd "$(dirname "$0")/.."

SIM="$(mktemp -u /tmp/ium-crlf-sim-XXXXXX)"
echo "== clone HEAD with core.autocrlf=true (the windows default) =="
git clone -q --no-hardlinks . "$SIM" -c core.autocrlf=true
cd "$SIM"

echo "== byte-divergence: disk (CRLF checkout) vs committed blobs =="
diverge=0; total=0
while IFS=$'\t' read -r path blob; do
  total=$((total + 1))
  # committed bytes
  git cat-file blob "$blob" > /tmp/blob.$$ 2>/dev/null || continue
  if ! cmp -s /tmp/blob.$$ "$path"; then
    diverge=$((diverge + 1))
    [ "$diverge" -le 5 ] && echo "  DIVERGES: $path"
  fi
  rm -f /tmp/blob.$$
done < <(git ls-tree -r HEAD --format='%(path)%(objectname)' | sed 's/\(.*\)\([0-9a-f]\{40\}\)$/\1\t\2/')
echo "byte-diverging files: $diverge / $total tracked"

echo "== the lane-6 failure class: \$-anchored regex on the CRLF sheet =="
python3 - <<'EOF'
import re
sheet = open("docs/10_Brand_Sheet.md", "rb").read().decode("utf-8")
rows = re.findall(r"^\| `(--[a-z0-9-]+)` \| `(#[0-9a-fA-F]{6})` \| `(#[0-9a-fA-F]{6})` \|$",
                  sheet, re.MULTILINE)
print(f"  CRLF copy: sheetTable parsed {len(rows)} rows (Linux gate parses 22)")
assert len(rows) == 0, "expected the CRLF copy to parse ZERO rows"
print("  RED confirmed: the exact run-42277434539 failure, reproduced from a plain windows-default clone")
EOF

echo "== sanity: the SAME clone on Linux CI (autocrlf unset here) stays LF =="
cd /home/z/my-project
SIM2="$(mktemp -u /tmp/ium-lf-sim-XXXXXX)"
git clone -q --no-hardlinks . "$SIM2"
n=$(cd "$SIM2" && python3 -c "
import subprocess
out = subprocess.check_output(['git','ls-files','--eol']).decode()
print(sum(1 for l in out.splitlines() if l.startswith('i/lf') and 'w/crlf' in l))
")
echo "  LF clone: $n working-tree CRLF files (expected 0)"
rm -rf "$SIM" "$SIM2"
