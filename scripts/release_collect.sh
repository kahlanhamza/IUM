#!/usr/bin/env bash
# Collect the per-OS installer artifacts into dist_release/ with measured
# sizes + sha256 sidecars (#65 bar 2: "budgeted and MEASURED — numbers, not
# vibes"). Run from ium-app/ after `tauri build`. Prints a Markdown table of
# the collected artifacts on stdout for the release notes.
set -euo pipefail

OUT="dist_release"

# The bundle dir follows the cargo workspace layout: with a workspace root
# target/ it is ../target/release/bundle (what CI proved on the first run —
# "no installer artifacts found under src-tauri/target/release/bundle");
# without one, the tauri-local path applies. Locate, never assume.
BUNDLE="$(ls -d ../target/release/bundle 2>/dev/null || ls -d src-tauri/target/release/bundle 2>/dev/null || true)"
if [ -z "${BUNDLE:-}" ]; then
  echo "ERROR: no release bundle dir found (looked at ../target and src-tauri/target)" >&2
  exit 1
fi
echo "bundle dir: $BUNDLE"

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
# the updater bundles + .sig sidecars (#80): tauri emits them next to the
# installers when createUpdaterArtifacts is on; the manifest builder pairs
# them in build_latest_json.py — an unsigned bundle never ships
ARTIFACTS+=("$BUNDLE"/msi/*.msi.zip*)
ARTIFACTS+=("$BUNDLE"/nsis/*.nsis.zip*)
ARTIFACTS+=("$BUNDLE"/appimage/*.AppImage.tar.gz*)
ARTIFACTS+=("$BUNDLE"/macos/*.app.tar.gz*)

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
  for f in ./*.deb ./*.rpm ./*.AppImage ./*.dmg ./*.msi ./*.exe \
           ./*.msi.zip ./*.nsis.zip ./*.AppImage.tar.gz ./*.app.tar.gz; do
    [ -f "$f" ] || continue
    bytes=$(stat -c %s "$f" 2>/dev/null || stat -f %z "$f")
    human=$(numfmt --to=iec --suffix= "$bytes" 2>/dev/null || echo "$bytes B")
    echo "| ${f#./} | $bytes | $human |"
  done
)
