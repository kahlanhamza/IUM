#!/usr/bin/env bash
# Regenerate the v0.2.0 mid-flight journal fixture for the upgrade-path pin (#96).
#
# Provenance contract: the fixture bytes are written by the v0.2.0 tag's own
# code — a git worktree at v0.2.0, the generator copied in from
# scripts/upgrade_fixture/, driven through the REAL v0.2.0 JobManager, then
# aborted mid-call. The worktree is discarded; this script and the generator
# at HEAD are the only provenance needed to reproduce the bytes.
#
# Usage: bash scripts/upgrade_fixture.sh [workdir]
set -euo pipefail
cd "$(dirname "$0")/.."

WORK="${1:-/tmp/ium-v020-fixture}"
OUT="ium-core/tests/fixtures/upgrade_v020_journal.db"
GEN="scripts/upgrade_fixture/gen_upgrade_fixture.rs"

[ -f "$GEN" ] || { echo "generator missing: $GEN" >&2; exit 1; }

echo "== worktree at v0.2.0 =="
git worktree remove --force "$WORK" 2>/dev/null || true
git worktree add "$WORK" v0.2.0

echo "== copy the generator into the worktree =="
mkdir -p "$WORK/ium-core/tests"
cp "$GEN" "$WORK/ium-core/tests/gen_upgrade_fixture.rs"

echo "== build + run against v0.2.0 (fresh target: this is the point) =="
mkdir -p /tmp/ium-upgrade-fixture
rm -f /tmp/ium-upgrade-fixture/journal.db*
(
  cd "$WORK"
  . "$(pwd)/../scripts/rust-env.sh" 2>/dev/null \
    || . "$(git rev-parse --show-toplevel 2>/dev/null || echo /home/z/my-project)/scripts/rust-env.sh"
  IUM_UPGRADE_FIXTURE_DIR=/tmp/ium-upgrade-fixture \
    cargo test -p ium-core --test gen_upgrade_fixture -- --nocapture
)

echo "== snapshot the journal into the fixtures dir =="
cp /tmp/ium-upgrade-fixture/journal.db "$OUT"
SHA=$(sha256sum "$OUT" | cut -d' ' -f1)
echo "fixture: $OUT"
echo "sha256:  $SHA"

echo "== cleanup =="
git worktree remove --force "$WORK"
rm -rf /tmp/ium-upgrade-fixture
echo "done — paste the sha256 into the HEAD test header and commit the fixture"
