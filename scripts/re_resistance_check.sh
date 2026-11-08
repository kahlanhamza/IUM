#!/usr/bin/env bash
# re_resistance_check.sh — Lane 8 of #66 (#76): the amendment pins, verified.
#
# The owner amendment (issue #66, comment 6673366729) restored RE-resistance
# as a real bar: at ship the repo goes CLOSED, and a closed release must add
# (1) release builds stripped of debug symbols, (2) no panic paths printing
# internal paths, (3) secrets never embedded in the binary, (4) a final
# RE-review checklist before the private flip. NOT planned: aggressive
# obfuscation (breaks crash triage).
#
# Modes:
#   --selftest          synthetic detector proofs (CI: Linux step, every push)
#   --repo [root]       the committed-tree pins: [profile.release] strip,
#                       the checklist doc, the source panic-path audit
#   --bin <path>        the release-time binary gate: stripped verification,
#                       secret-value scan, build-machine path scan
#
# Why value-shaped patterns only: cookie/param NAMES (ndus, csrfToken,
# jsToken, browserid) are protocol vocabulary the adapters legitimately
# contain; what must never be embedded is a credential VALUE. The env
# contract (*_from_env + the vault) guarantees that; this gate verifies it
# on the built binary instead of trusting it.
set -euo pipefail

# Credential VALUE shapes (grep -aE, one pattern per line).
SECRET_PATTERNS=(
  'github_pat_[A-Za-z0-9_]{20,}'
  'gh[pous]_[A-Za-z0-9]{20,}'
  'AKIA[0-9A-Z]{16}'
  'sk-[A-Za-z0-9_-]{20,}'
  'xox[bpas]-[A-Za-z0-9-]{10,}'
  '-----BEGIN [A-Z ]*PRIVATE KEY'
  'ndus=[A-Za-z0-9%_-]{8,}'
  'browserid=[A-Za-z0-9]{8,}'
  'csrfToken=[A-Za-z0-9]{8,}'
  'jsToken=[A-Za-z0-9]{8,}'
)
# Build-machine path fragments — no build host leaks into the shipped binary.
BUILD_PATHS=( '/home/' '/root/' '/Users/' 'C:\Users\' )

# exit 1: the detectors abort the script on any verdict failure. The
# selftest proves expected failures by calling them in SUBSHELLS — an exit
# there is just the subshell's status, never the script's.
fail() { echo "RE-CHECK FAIL: $*"; exit 1; }

# scan_file <path> — secret + build-path value scan. NEVER prints the
# matched values (a scanner that echoes secrets is itself a leak): reports
# pattern names + hit counts only. Returns non-zero when anything is found.
scan_file() {
  local bin="$1" hits="" p n
  for p in "${SECRET_PATTERNS[@]}"; do
    # `|| true` — with pipefail, a no-match grep (exit 1) would kill this
    # assignment under set -e and the whole scan would die silently.
    n="$(grep -aoE "$p" "$bin" 2>/dev/null | wc -l || true)"
    if [ "$n" -gt 0 ]; then
      hits="$hits\n  secret-value pattern: $p ($n hit(s))"
    fi
  done
  for p in "${BUILD_PATHS[@]}"; do
    if grep -aqF "$p" "$bin" 2>/dev/null; then
      hits="$hits\n  build-path: $p"
    fi
  done
  if [ -n "$hits" ]; then
    printf "RE-CHECK FAIL: %s contains embedded material it must not carry:$hits\n" "$bin"
    return 1
  fi
  echo "RE-CHECK: $bin — no secret values, no build-machine paths"
}

# check_stripped <path> — debug-symbol verification. `file` says "stripped"
# on unix; the RSDS codeview marker is the windows tell. Capability-gated:
# a loud skip when the detection tooling is absent (enforced where present).
check_stripped() {
  local bin="$1"
  if command -v file >/dev/null 2>&1; then
    local desc
    desc="$(file -b "$bin")"
    case "$desc" in
      # negative verdicts FIRST — "not stripped" contains the substring
      # "stripped", so the bare pattern must not win the match order
      *not\ stripped*) fail "$bin is NOT stripped: $desc" ;;
      *"with debug_info"*) fail "$bin carries debug_info: $desc" ;;
      *stripped*) echo "RE-CHECK: $bin — symbols stripped ($desc)" ;;
      *)
        echo "RE-CHECK: $bin — file verdict inconclusive ('$desc'), falling back"
        fallback_stripped "$bin"
        ;;
    esac
  else
    echo "RE-CHECK: file(1) unavailable — capability-gated skip, loud"
    fallback_stripped "$bin"
  fi
}

fallback_stripped() {
  local bin="$1"
  if command -v nm >/dev/null 2>&1; then
    # The verdict is the SYMBOL COUNT, not the exit status: BSD nm (macOS)
    # exits 0 even for a stripped binary — "no name list" goes to stderr
    # and stdout stays empty — so exit-status-only checking false-positives
    # on every honestly-stripped Mach-O (release #13 proved it: strip(1)
    # succeeded and the count-based verdict still failed it).
    if [ "$(nm "$bin" 2>/dev/null | wc -l)" -gt 0 ]; then
      fail "$bin still lists symbols via nm"
    fi
    echo "RE-CHECK: $bin — zero symbols via nm — stripped"
  elif grep -aq RSDS "$bin" 2>/dev/null; then
    fail "$bin carries the RSDS codeview marker (debug data present)"
  else
    echo "RE-CHECK: $bin — no RSDS marker — stripped (windows fallback)"
  fi
}

# repo_pins_check <root> — the committed-tree pins (amendment items 1, 2, 4).
repo_pins_check() {
  # NOTE: one `local` per assignment — a single `local root="$1" cargo="$root/…"`
  # expands $root before root is assigned (unbound under set -u).
  local root="$1"
  local cargo="$root/Cargo.toml"
  local doc="$root/docs/12_RE_Resistance_Checklist.md"
  grep -q '^\[profile.release\]' "$cargo" || fail "no [profile.release] pin in root Cargo.toml (amendment item 1)"
  grep -qE 'strip[[:space:]]*=[[:space:]]*"(symbols|debuginfo)"|strip[[:space:]]*=[[:space:]]*true' "$cargo" \
    || fail "[profile.release] has no strip pin in root Cargo.toml"
  echo "RE-CHECK: [profile.release] strip pin present"

  [ -f "$doc" ] || fail "the RE-review checklist doc is missing ($doc) (amendment item 4)"
  local h
  for h in "stripped of debug symbols" "no panic paths print internal paths" \
           "secrets never embedded" "the private-flip procedure"; do
    grep -qi "$h" "$doc" || fail "checklist doc is missing the section: $h"
  done
  echo "RE-CHECK: checklist doc carries all four amendment pins"

  # The source panic audit (amendment item 2): no panic/expect/unwrap
  # message interpolates a filesystem path (.display() / {:?} of a Path).
  local audit
  audit="$(grep -rnE '(panic!|\.expect\(|\.unwrap\()' "$root/ium-core/src" "$root/ium-app/src-tauri/src" 2>/dev/null \
    | grep -E '\.display\(\)|\{:\?\}' || true)"
  if [ -n "$audit" ]; then
    fail "panic-path audit: path interpolation in panic messages:$audit"
  fi
  echo "RE-CHECK: source panic audit clean (no path interpolation in panic messages)"
}

selftest() {
  # GLOBAL, not local: the EXIT trap (set below) fires at script exit,
  # after this function's locals are gone — a local here dies as
  # "unbound variable" inside the trap and flips the exit code.
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "== RE-CHECK selftest: synthetic verdict proofs =="

  # 1. every planted credential VALUE is detected; the clean fixture passes.
  # The planted value is ASSEMBLED at runtime: a token-shaped string must
  # never sit in a source file (the pre-commit guard would — correctly —
  # block the commit; the scanner only needs the shape, not the bytes).
  fake_prefix='github_pat_'
  printf 'token=%sABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 YZ\n' "$fake_prefix" > "$tmp/dirty"
  printf 'this is a clean binary payload with no credentials at all\n' > "$tmp/clean"
  (scan_file "$tmp/dirty") >/dev/null 2>&1 && fail "selftest: planted github_pat_ NOT detected"
  (scan_file "$tmp/clean") >/dev/null || fail "selftest: clean fixture flagged"
  printf 'cookie: ndus=AbCdEfGh12345678; other=x\n' > "$tmp/dirty2"
  (scan_file "$tmp/dirty2") >/dev/null 2>&1 && fail "selftest: planted ndus cookie NOT detected"
  # protocol vocabulary without a value must NOT be flagged
  printf 'params: csrfToken & jsToken & ndus names only\n' > "$tmp/names"
  (scan_file "$tmp/names") >/dev/null || fail "selftest: protocol NAMES wrongly flagged"
  echo "  secrets scanner: plants detected, clean passes, names pass"

  # 2. the stripped detector flags a symbols-present binary and passes a
  #    stripped copy (capability-gated: rustc + file + strip must exist).
  if command -v rustc >/dev/null 2>&1 && command -v file >/dev/null 2>&1 && command -v strip >/dev/null 2>&1; then
    printf 'fn main() { println!("ium re selftest"); }\n' > "$tmp/hello.rs"
    rustc -O "$tmp/hello.rs" -o "$tmp/hello" 2>/dev/null
    (check_stripped "$tmp/hello") >/dev/null 2>&1 && fail "selftest: unstripped binary NOT flagged"
    strip "$tmp/hello"
    (check_stripped "$tmp/hello") >/dev/null || fail "selftest: stripped binary flagged as unstripped"
    echo "  stripped detector: unstripped flagged, stripped passes"
  else
    echo "  stripped detector: SKIPPED LOUD (rustc/file/strip absent) — enforced where the toolchain exists"
  fi

  # 3. the repo-pins detector rejects a tree without the pins and accepts a
  #    complete one.
  mkdir -p "$tmp/badrepo/docs"
  printf '[workspace]\nresolver = "2"\n' > "$tmp/badrepo/Cargo.toml"
  (repo_pins_check "$tmp/badrepo") >/dev/null 2>&1 && fail "selftest: pinless repo NOT rejected"
  mkdir -p "$tmp/goodrepo/docs/ium-core/src" "$tmp/goodrepo/ium-app/src-tauri/src"
  printf '[workspace]\n\n[profile.release]\nstrip = "symbols"\n' > "$tmp/goodrepo/Cargo.toml"
  printf '# checklist\n\n- release builds stripped of debug symbols\n- no panic paths print internal paths\n- secrets never embedded\n- the private-flip procedure\n' > "$tmp/goodrepo/docs/12_RE_Resistance_Checklist.md"
  (repo_pins_check "$tmp/goodrepo") >/dev/null || fail "selftest: complete repo rejected"
  echo "  repo-pins detector: pinless rejected, complete accepted"

  echo "RE-SELFTEST PASS: secrets / stripped / repo-pins"
}

case "${1:-}" in
  --selftest)
    selftest
    ;;
  --repo)
    repo_pins_check "${2:-$(cd "$(dirname "$0")/.." && pwd)}"
    echo "RE-CHECK: repo pins GREEN"
    ;;
  --bin)
    bin="${2:?usage: re_resistance_check.sh --bin <path>}"
    [ -f "$bin" ] || fail "no binary at $bin"
    check_stripped "$bin"
    scan_file "$bin"
    echo "RE-CHECK: binary gate GREEN for $bin"
    ;;
  *)
    fail "usage: re_resistance_check.sh --selftest | --repo [root] | --bin <path>"
    ;;
esac
