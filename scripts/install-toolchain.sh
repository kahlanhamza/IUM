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
  exit 0
fi

mkdir -p "$ROOT/.toolchain"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$ROOT/.toolchain/rustup-init.sh"
sh "$ROOT/.toolchain/rustup-init.sh" -y --profile minimal --default-toolchain stable
# Gates need rustfmt + clippy; minimal profile ships neither (recycle on
# 2026-11-02 cost a bootstrap detour). Install them with the toolchain.
rustup component add rustfmt clippy
cargo --version
rustc --version
echo "[install-toolchain] done — activate with: . scripts/rust-env.sh"
