"""Post the LANDED evidence comment on #76."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## LANDED — the RE-resistance bar shipped and CI-green (lane 8 of #66 complete — the product bar is DONE)

**The loop, on the record:**

- **RED** (comment 6699954232, at HEAD `1c0c041`): `--repo` failed with named reasons — no `[profile.release]` pin, no checklist doc. Issue filed BEFORE implementation.
- **Landed in one commit (`e05a903`, auto-closed this issue):**
  - **Pin 1 — stripped releases**: `[profile.release] strip = "symbols"` in the root Cargo.toml (workspace-wide). Cargo's default was `strip = "none"` — the symbol table shipped until now.
  - **Pin 2 — no panic paths print internal paths**: the source audit rides `--repo` on every push (fails on any panic!/expect!/unwrap! message interpolating `.display()`/`{:?}`); clean today; the telemetry-panic-location distinction documented (compile-time workspace-relative, LOCAL buffer — not a user or build-machine path).
  - **Pin 3 — secrets never embedded**: `--bin` scans the built binary for credential VALUE shapes (GitHub/AWS/`sk-`/Slack/PEM/TeraBox cookie values), NEVER printing matched values; protocol NAMES are vocabulary and the selftest proves the distinction.
  - **Pin 4 — no build-machine paths**: the same `--bin` scan; this pin caught the first real leak.
  - **`docs/12_RE_Resistance_Checklist.md`** — the final flip checklist: the four pins (mechanism + verification + status) + the ordered private-flip procedure (freeze/tag → burn check → secrets inventory → checklist read-off → flip → post-flip CI note).
  - **Wiring**: ci.yml core job (Linux step, no new job — the CI-economics rule) runs `--selftest` + `--repo` on every push; release.yml exports the registry remap into RUSTFLAGS and runs `--bin` per-OS after build, BEFORE collect/upload — a FAIL stops the release.

**The gate's first real catch (why this lane exists):** the first release-profile build leaked `/home/` into the binary — registry-dependency panic locations embed the build host's ABSOLUTE registry path as `Location` strings. Stable fix: `scripts/rustflags_remap.sh` computes the CARGO_HOME registry prefix the way cargo resolves it (env override first — NOT the rustup sysroot; the first cut was wrong and the gate caught the surviving `/home/` a second time); release.yml wires it into RUSTFLAGS. `cargo`'s `trim-paths` profile option is nightly-only on the stable toolchain — tried and rejected. After the remap the SAME binary: **stripped ✓, no secret values ✓, no build paths ✓ — binary gate GREEN on a real release-profile build.**

**Gates earned their keep — seven defects caught pre-ship:** six in the gate script itself (unbound `local` under `set -u`; `fail()` exit-vs-return context; a `case` order where `*stripped*` matched "not stripped"; a `pipefail` silent kill on no-match grep; a function-local temp dying inside the EXIT trap; the CARGO_HOME-from-sysroot misderivation) + one in the product surface (the `/home/` registry-path leak). The pre-commit token guard also fired correctly: the token-shaped fixture was blocked from history → the selftest now ASSEMBLES the fixture value at runtime.

**CI run 42322157075 on `e05a903`: 8/8 GREEN, all three OSes.** The `--bin` gate's per-OS release verdict lands at the next tag — same honesty contract as app-e2e.

**Product-bar #66: lanes 1–8 COMPLETE** (#67 primitives+a11y, #69 settings+vault UI, #70 error copy, #71 SBOM/provenance, #72 load e2e+size budget, #73 brand sheet, #75 telemetry stance, #76 RE-resistance) + #74 repo hardening. Standing rules remain: #65 (release bar — per-OS installers, signing = pinned owner ask) and #66 stay open as the standing bar; #59/#60 (Drive/YouTube lanes) are the next engine work.
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/76/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("landing comment posted:", c["id"])
