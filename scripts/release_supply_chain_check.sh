#!/usr/bin/env bash
# Lane 4 (#71): verify a published release carries SBOM + provenance.
#
# Usage: release_supply_chain_check.sh <tag>
#
# Checks (pure REST API — no `gh` dependency, works from the sandbox):
#   1. the release exists for <tag>
#   2. at least 2 CycloneDX SBOM assets (*.cdx.json), each parsing with
#      specVersion 1.x and >= 100 components
#   3. EVERY installer asset's sha256 resolves to at least one build
#      provenance attestation in the repo's attestation store
#      (GET /repos/{owner}/{repo}/attestations/{digest})
#
# Exit 0 = release passes the supply-chain bar. Exit 1 = FAIL with reason.
set -euo pipefail

REPO="kahlanhamza/IUM"
TAG="${1:?usage: release_supply_chain_check.sh <tag>}"
TOKEN="$(cat /home/z/my-project/.secrets/gh_token)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

api() { # api <path> [curl args...]
  curl -sS -H "Authorization: Bearer $TOKEN" \
    -H "Accept: application/vnd.github+json" "$@"
}

fail() { echo "FAIL: $*"; exit 1; }

echo "== supply-chain check: $REPO @ $TAG =="

# --- 1. release exists -------------------------------------------------------
REL_JSON="$TMP/release.json"
api "https://api.github.com/repos/$REPO/releases/tags/$TAG" > "$REL_JSON"
python3 - "$REL_JSON" <<'PY' || fail "no release for tag $TAG (or API error) — see above"
import json, sys
d = json.load(open(sys.argv[1]))
if "assets" not in d:
    raise SystemExit(1)
PY
ASSETS_JSON="$TMP/assets.json"
python3 - "$REL_JSON" "$ASSETS_JSON" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
json.dump(d["assets"], open(sys.argv[2], "w"))
print(f"release found: {d.get('name') or d.get('tag_name')}, {len(d['assets'])} assets")
PY

# --- 2. SBOM assets present and honestly populated ---------------------------
SBOM_NAMES=$(python3 - "$ASSETS_JSON" <<'PY'
import json, sys
assets = json.load(open(sys.argv[1]))
sboms = [a for a in assets if a["name"].endswith(".cdx.json")]
print("\n".join(a["name"] for a in sboms))
PY
) || true
SBOM_COUNT=$(echo -n "$SBOM_NAMES" | grep -c 'cdx.json' || true)
echo "SBOM assets: $SBOM_COUNT"
[ "$SBOM_COUNT" -ge 2 ] || fail "expected >= 2 *.cdx.json SBOM assets (rust + app), found $SBOM_COUNT"

# every SBOM must parse as CycloneDX with a real dependency graph
python3 - "$ASSETS_JSON" "$TOKEN" "$TMP" <<'PY'
import json, subprocess, sys, urllib.request
assets = json.load(open(sys.argv[1]))
token, tmp = sys.argv[2], sys.argv[3]
sboms = [a for a in assets if a["name"].endswith(".cdx.json")]
for a in sboms:
    path = f"{tmp}/{a['name']}"
    req = urllib.request.Request(a["browser_download_url"],
                                 headers={"Authorization": f"Bearer {token}"})
    open(path, "wb").write(urllib.request.urlopen(req).read())
    d = json.load(open(path))  # raises on non-JSON
    spec = d.get("specVersion", "")
    n = len(d.get("components", []))
    print(f"  {a['name']}: specVersion={spec}, components={n}")
    if not spec.startswith("1."):
        sys.exit(f"FAIL: {a['name']} is not CycloneDX 1.x (specVersion={spec!r})")
    if n < 100:
        sys.exit(f"FAIL: {a['name']} has only {n} components (< 100) — "
                 "an SBOM this small is not the honest dependency graph")
print("SBOM content: OK")
PY

# --- 3. provenance: every installer attested ---------------------------------
INSTALLERS=$(python3 - "$ASSETS_JSON" <<'PY'
import json, sys
assets = json.load(open(sys.argv[1]))
exts = (".deb", ".rpm", ".AppImage", ".dmg", ".msi")
names = [a["name"] for a in assets
         if a["name"].endswith(exts) or a["name"].endswith("-setup.exe")]
print("\n".join(names))
PY
) || true
[ -n "$INSTALLERS" ] || fail "no installer assets found on the release"

for name in $INSTALLERS; do
  URL=$(python3 -c "
import json, sys
for a in json.load(open('$ASSETS_JSON')):
    if a['name'] == '$name':
        print(a['browser_download_url'])")
  FILE="$TMP/$name"
  echo "verifying provenance: $name"
  curl -sSL -H "Authorization: Bearer $TOKEN" -o "$FILE" "$URL"
  DIGEST=$(sha256sum "$FILE" | cut -d' ' -f1)
  # The API wants the digest ALGORITHM-PREFIXED ("sha256:<hex>") — bare hex
  # always 404s (false negative proven live on the v0.1.1 attestations).
  CODE=$(curl -sS -o "$TMP/att.json" -w "%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    "https://api.github.com/repos/$REPO/attestations/sha256:$DIGEST")
  [ "$CODE" = "200" ] || fail "$name (sha256:$DIGEST) has NO build provenance attestation (HTTP $CODE)"
  N=$(python3 -c "import json; print(len(json.load(open('$TMP/att.json')).get('attestations', [])))")
  [ "$N" -ge 1 ] || fail "$name attestation lookup returned 0 entries"
  echo "  attested: yes ($N attestation(s))"
done

echo "PASS: $TAG carries SBOMs (>=2) and every installer is attested."
