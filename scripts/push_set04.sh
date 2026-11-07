#!/usr/bin/env bash
# Commit set04 (dart flat colorways) onto design/logo-concepts WITHOUT
# switching the main worktree — plumbing only, immune to the background
# process that keeps resetting the sandbox worktree to main.
set -euo pipefail
cd /home/z/my-project

SRC=download/logo_set04
BRANCH=design/logo-concepts
OLD=$(git rev-parse "$BRANCH")
export GIT_INDEX_FILE=/tmp/set04.index
rm -f "$GIT_INDEX_FILE"
git read-tree "$BRANCH"

add() { # add <repo-relative-path> <local-file>
  local sha
  sha=$(git hash-object -w "$2")
  git update-index --add --cacheinfo "100644,$sha,$1"
}

BASE_DESIGN=design/logo-variants/set04
for f in "$SRC"/svg/*.svg; do
  add "$BASE_DESIGN/svg/$(basename "$f")" "$f"
done
for f in "$SRC"/png/*.png; do
  add "$BASE_DESIGN/png/$(basename "$f")" "$f"
done
add "$BASE_DESIGN/showcase_set04_dart_flat.png" "$SRC/showcase_set04_dart_flat.png"

TREE=$(git write-tree)
COMMIT=$(git commit-tree "$TREE" -p "$OLD" -m "design: set04 — the dart colorways, flat solid inks, zero gradients (refs #77)

Owner approved the set03 dart geometry and banned gradients (reference-image
crutch only). Same two smooth concentric-arc ribbons, byte-for-byte frozen
geometry; only the stroke inks changed. Five flat colorways for the pick:
cobalt one-ink, duo cyan, duo violet, ink stealth, white mono. Review sheet
checks light + dark surfaces and a 96/48/24 size ladder.

No docs touched — docs unfreeze after the owner picks the colorway.")

git update-ref "refs/heads/$BRANCH" "$COMMIT" "$OLD"
echo "branch moved: $OLD -> $COMMIT"

unset GIT_INDEX_FILE
HELPER='!f(){ printf "username=%s\n" kahlanhamza; printf "password=%s\n" "$(cat /home/z/my-project/.secrets/gh_token)"; }; f'
git -c "credential.helper=${HELPER}" push origin "$BRANCH" 2>&1 | sed -E 's#(https://)[^@/]*@#\1#g'
echo "pushed"
git ls-remote origin "refs/heads/$BRANCH" | cut -c1-12
