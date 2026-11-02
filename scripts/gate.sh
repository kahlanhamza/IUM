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

# ---- public API docs lane (issue #19, M5 API freeze) -----------------------
# The frozen surface must stay documented: missing_docs is #![deny] in lib.rs
# (clippy enforces it above); rustdoc must build warning-free.
echo "== gate: cargo doc (warnings as errors) =="
RUSTDOCFLAGS="-D warnings" cargo doc -p ium-core --no-deps --quiet

# ---- ium-app lane (ADR-006 amendment, issue #14) --------------------------
# The frontend lane (tsc strict + vite build) runs EVERYWHERE and is
# unconditional. The Rust shell lane needs platform libraries: native where
# they exist (WebKitGTK on Linux dev machines, none on Windows/macOS), and is
# otherwise owned by the CI `ium-app` job on all 3 OS — a capability-gated
# local skip is LOUD here and enforced there, never silent (same honesty
# contract as the env-gated S3/TUS live suites).
echo "== gate: ium-app frontend lane =="
( cd ium-app && npm ci --no-audit --no-fund && npm run build && npm test )

if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists webkit2gtk-4.1; then
  echo "== gate: ium-app rust lane (native, WebKitGTK present) =="
  cargo clippy -p ium-app --all-targets -- -D warnings
  cargo test -p ium-app
elif [ "$(uname -s)" = "Linux" ]; then
  echo "== gate: ium-app rust lane SKIPPED — WebKitGTK dev libs absent and this"
  echo "        sandbox has no root to install them; the CI 'ium-app' job owns"
  echo "        this lane natively on ubuntu/macos/windows for every push."
else
  echo "== gate: ium-app rust lane SKIPPED — platform dev libs not detected;"
  echo "        the CI 'ium-app' job owns this lane for every push."
fi

echo "== gate: GREEN on committed tree $HEAD_SHA =="
