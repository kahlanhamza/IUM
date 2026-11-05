#!/usr/bin/env bash
# Collect the per-OS installer artifacts into dist_release/ with measured
# sizes + sha256 sidecars (#65 bar 2: "budgeted and MEASURED — numbers, not
# vibes"). Run from ium-app/ after `tauri build`. Prints a Markdown table of
# the collected artifacts on stdout for the release notes.
set -euo pipefail

OUT="dist_release"
BUNDLE="src-tauri/target/release/bundle"
rm -rf "$OUT"
mkdir -p "$OUT"

# Per-OS globs — copy whatever exists (the release job builds only its
# platform's bundles; the globs are the honest union).
shopt -s nullglob
ARTIFACTS=()
ARTIFACTS+=("$BUNDLE"/deb/*.deb)
ARTIFACTS+=("$BUNDLE"/rpm/*.rpm)
ARTIFACTS+=("$BUNDLE"/appimage/*.AppImage)
ARTIFACTS+=("$BUNDLE"/dmg/*.dmg)
ARTIFACTS+=("$BUNDLE"/msi/*.msi)
ARTIFACTS+=("$BUNDLE"/nsis/*.exe)

if [ ${#ARTIFACTS[@]} -eq 0 ]; then
  echo "ERROR: no installer artifacts found under $BUNDLE" >&2
  exit 1
fi

for f in "${ARTIFACTS[@]}"; do
  cp -v "$f" "$OUT/"
done

# sha256 sidecar + the measured-sizes table for the release notes.
(
  cd "$OUT"
  sha256sum ./* | sed 's|\./||' > SHA256SUMS.txt
  echo "| Installer | Size (bytes) | Size |"
  echo "|---|---|---|"
  for f in ./*.deb ./*.rpm ./*.AppImage ./*.dmg ./*.msi ./*.exe; do
    [ -f "$f" ] || continue
    bytes=$(stat -c %s "$f" 2>/dev/null || stat -f %z "$f")
    human=$(numfmt --to=iec --suffix= "$bytes" 2>/dev/null || echo "$bytes B")
    echo "| ${f#./} | $bytes | $human |"
  done
)
