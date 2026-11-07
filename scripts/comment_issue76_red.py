"""Post the RED + real-catch evidence comment on #76."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## RED evidence + the gate's first real catch (at HEAD `1c0c041`, 2026-11-07)

**RED (repo pins at HEAD):** `scripts/re_resistance_check.sh --repo` fails with named reasons —
- `RE-CHECK FAIL: no [profile.release] pin in root Cargo.toml (amendment item 1)`
- (and) the checklist doc is missing (amendment item 4)

**The self-test (all detector verdicts, proven):** `RE-SELFTEST PASS: secrets / stripped / repo-pins` — planted credential values detected, clean and name-only fixtures pass, an unstripped rustc-compiled binary is flagged and its stripped copy passes, a pinless fixture repo is rejected and a complete one accepted. The gate's own build caught **six real defects in itself pre-ship** (unbound `local` under `set -u`; `fail()` exit-vs-return context; a `case` order where `*stripped*` matched "not stripped"; a `pipefail` silent kill on no-match grep; a function-local temp dir dying inside the EXIT trap; the registry prefix derived from the rustup sysroot instead of CARGO_HOME — the remap silently matched nothing).

**The first real catch (the lane's whole point):** a real release-profile build (`cargo build --release -p ium-core`, the drill-child binary) — stripped ✓, no secret values ✓ — **failed the build-path scan: `/home/` is embedded in the binary.** Extraction shows registry-dependency panic locations carrying the build host's ABSOLUTE registry path (`/home/z/my-project/.toolchain/cargo/registry/src/index.crates.io-…/<crate>/src/…`) as `Location` strings. Every release binary would have shipped the build host's home directory.

**The fix (stable, verified):** `cargo`'s `trim-paths` profile option is nightly-only on the stable toolchain (would break every CI build — tried and rejected). The stable fix: `scripts/rustflags_remap.sh` computes the CARGO_HOME registry prefix the way cargo resolves it (env override, else the platform default — NOT the rustup sysroot; first cut got this wrong and the gate caught the surviving `/home/` again) and emits the separator variants; release.yml exports it into RUSTFLAGS on the build step. After the remap, the SAME real binary: **stripped ✓, no secret values ✓, no build-machine paths ✓ — binary gate GREEN.**

Next: the profile pin + checklist doc + ci.yml (self-test + repo pins on every push) + release.yml (remap + `--bin` gate before upload) land as one commit; `--repo` GREEN; full gates; CI 3-OS.
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/76/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("evidence comment posted:", c["id"])
