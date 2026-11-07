#!/usr/bin/env bash
# rustflags_remap.sh — echo the --remap-path-prefix flag(s) for THIS machine's
# cargo registry (lane 8 #76).
#
# Why: registry-dependency panic locations embed the ABSOLUTE registry path
# (/home/…/.cargo/registry/..., /Users/…, C:\Users\…) into every release
# binary as Location strings — caught by re_resistance_check.sh --bin on the
# first real release-profile build. Remapping the registry prefix to
# /registry keeps the crate-relative tail (honest triage) and kills the
# build-host leak on every machine.
#
# CARGO_HOME is taken from the environment exactly the way cargo resolves
# it (env override, else the platform default next to $HOME) — NOT from the
# rustup sysroot: with rustup, the sysroot lives under RUSTUP_HOME, which is
# a different tree than CARGO_HOME (the first cut got this wrong and
# remapped a prefix that never matched — the gate caught the surviving
# /home/ strings).
#
# Separator variants: rustc on windows embeds backslash paths while git-bash
# computes forward ones — both (and the all-backslash form of a mixed env
# value) are emitted; an unmatched prefix is loud: the --bin gate FAILS the
# release and names the pattern that leaked.
set -euo pipefail

if [ -n "${CARGO_HOME:-}" ]; then
  base="$CARGO_HOME"
elif [ -n "${HOME:-}" ]; then
  base="$HOME/.cargo"
else
  echo "rustflags_remap: cannot resolve CARGO_HOME" >&2
  exit 1
fi

reg="${base%/}/registry"
flags=""
seen=" "
add() { # add <prefix>
  case "$seen" in *" $1 "*) ;; *) seen="$seen$1 "; flags="$flags --remap-path-prefix=$1=/registry" ;; esac
}
add "$reg"                 # as the environment provides it
add "${reg//\\//}"         # all forward slashes
add "${reg//\//\\}"        # all backslashes
echo "${flags# }"
