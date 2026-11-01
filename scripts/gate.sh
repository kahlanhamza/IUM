#!/usr/bin/env bash
# gate.sh — the non-bypassable quality gate, run against the COMMITTED tree.
#
# Defect #11 (2026-11-02): fix commit d62094a added the trybuild contract test
# but forgot to stage the ium-core/Cargo.toml dev-dependency line. The gates ran
# on the working tree and passed, while the pushed tree failed to compile on CI
# (E0433: cannot find crate `trybuild`). Lesson: what you push must be what you
# tested. This gate checks out HEAD in a detached worktree and runs fmt + clippy
# + the full test suite THERE.
#
# A dirty working tree is a hard refusal — commit first, then gate, then push.
#
# Usage: bash scripts/gate.sh        (sources the persistent toolchain itself)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/rust-env.sh

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "GATE FAIL: working tree is dirty." >&2
  echo "  Commit first, then gate: what you push is what you tested." >&2
  exit 1
fi

HEAD_SHA="$(git rev-parse --short HEAD)"
WT="$(mktemp -u /tmp/ium-gate-XXXXXX)"
git worktree add --detach "$WT" HEAD >/dev/null
cleanup() { git worktree remove --force "$WT" >/dev/null 2>&1 || true; }
trap cleanup EXIT

# Reuse dependency artifacts across gate runs (a fresh worktree has no target/).
export CARGO_TARGET_DIR=/home/z/my-project/.gate-target

echo "== gate: committed tree $HEAD_SHA =="
cargo fmt --all -- --check
cargo clippy -p ium-core --all-targets -- -D warnings
cargo test -p ium-core
echo "== gate: GREEN on committed tree $HEAD_SHA =="
