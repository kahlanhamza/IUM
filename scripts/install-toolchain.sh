#!/usr/bin/env bash
# Restores the pinned Rust toolchain into the PERSISTENT project-local directory.
# The sandbox recycles $HOME and /tmp; only /home/z/my-project survives.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export RUSTUP_HOME="$ROOT/.toolchain/rustup"
export CARGO_HOME="$ROOT/.toolchain/cargo"
export PATH="$CARGO_HOME/bin:$PATH"

if command -v cargo >/dev/null 2>&1 && cargo --version >/dev/null 2>&1; then
  echo "[install-toolchain] already present: $(cargo --version)"
else
  mkdir -p "$ROOT/.toolchain"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$ROOT/.toolchain/rustup-init.sh"
  sh "$ROOT/.toolchain/rustup-init.sh" -y --profile minimal --default-toolchain stable
  # Gates need rustfmt + clippy; minimal profile ships neither (recycle on
  # 2026-11-02 cost a bootstrap detour). Install them with the toolchain.
  rustup component add rustfmt clippy
  cargo --version
  rustc --version
fi

# The shared artifact cache (self-healing on every invocation — the recycle
# that wiped .toolchain on 2026-11-11 also wiped this file and the gate lost
# its warm dep cache). Every tree (main + gate worktrees) builds into ONE
# target dir: dep artifacts are content-addressed and path-independent, so a
# gate worktree pays only workspace-crate recompiles (gate.sh's "half the
# disk" contract). CARGO_TARGET_DIR stays unset in gate.sh; cargo falls back
# to this build.target-dir.
mkdir -p "$CARGO_HOME"
cat > "$CARGO_HOME/config.toml" <<EOF
[build]
target-dir = "$ROOT/target"

# The sandbox disk is 9.9G and a FULL-debuginfo test build of this workspace
# is ~6G — it fills the rootfs mid-link (ld SIGBUS, the 2026-11-11 crater)
# and CI-parity is preserved because line-tables-only keeps
# debug-assertions ON; only variable/type debuginfo is dropped.
[profile.dev]
debug = "line-tables-only"

[profile.test]
debug = "line-tables-only"
EOF
echo "[install-toolchain] done — activate with: . scripts/rust-env.sh"
