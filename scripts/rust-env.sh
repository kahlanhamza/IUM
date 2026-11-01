#!/bin/sh
# Source this to use the persistent sandbox toolchain: . scripts/rust-env.sh
export RUSTUP_HOME=/home/z/my-project/.toolchain/rustup
export CARGO_HOME=/home/z/my-project/.toolchain/cargo
export PATH="/home/z/my-project/.toolchain/cargo/bin:$PATH"
