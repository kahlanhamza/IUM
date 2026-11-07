"""File the lane-8 design note issue (RE-resistance at ship)."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

title = "RE-resistance at ship: the closed-release checklist, pinned and gated (Lane 8 of #66)"

body = """## Why

The owner amendment (comment 6673366729, 2026-11-06) restored RE-resistance as a real bar: at ship time the repo goes CLOSED, and the product must clear an honest reverse-engineering bar without aggressive obfuscation ("breaks crash triage; native code doesn't need it"). The amendment pins exactly what a closed release must add:

1. **release builds stripped of debug symbols**;
2. **no panic paths printing internal paths**;
3. **secrets never embedded in the binary** (the env-contract already guarantees this);
4. **a final RE-review checklist before the private flip**.

This issue builds the lane NOW (precedent: lane 5's installer-size budget gate was wired into release.yml + ci.yml before any release), so the private flip is a checklist read-off, not a scramble.

## Wire facts (executed at HEAD `1c0c041`, 2026-11-07)

- **No `[profile.release]` anywhere** (root Cargo.toml, ium-app/src-tauri, ium-core, ium-e2e): release builds ship with Cargo defaults — no DWARF debuginfo (default `debug = 0`) but the **symbol table intact** (`strip = "none"` default): function names recoverable by any `nm`/IDE.
- **The panic surface** (telemetry lane 7): the chained hook records payload message + `location().file()` (compile-time source path, workspace-relative) into the LOCAL buffer only — no transport exists; the default hook prints std-standard location. Our own panic/expect/unwrap messages need a source audit for interpolated FS paths (`.display()` / `{:?}` of PathBuf).
- **The secrets surface**: every credential enters via runtime env or the vault (`*_from_env` constructors: feishu `FEISHU_*`, rclone bridge `RCLONE_CONFIG_*`, terabox cookies, S3 `IUM_S3_*`, mega) — `apply_env` maps vault JSON to the exact env names; nothing credential-shaped is a compile-time literal. The gate VERIFIES this on the built binary instead of trusting it.
- **The binary name**: `productName: "IUM"` → `target/release/IUM[.exe]`, built per-OS in release.yml's build job before collect/smoke/upload.
- **The self-test precedent** (lane 5): `installer_size_gate.py --self-test` rides the ci.yml core job on Linux — synthetic verdict proofs, zero new jobs, seconds per run.
- **Telemetry overlap**: the panic event carries the source location by design (#75 charter, "The honest limits" — verbatim message + IUM source location). That is compile-time workspace-relative info, not a user-machine path; the checklist states this explicitly so the flip review doesn't misread it.

## Design

- **The profile pin** (amendment item 1): `[profile.release] strip = "symbols"` in the ROOT Cargo.toml (workspace-wide) — the shell binary, the drill child, e2e bins. No lto/panic changes (gold-plating; unwind stays for triage).
- **`scripts/re_resistance_check.sh`** — three modes:
  - `--selftest`: synthetic proofs — every secret prefix planted in a fixture is DETECTED; a clean fixture PASSES; a compiled-with-symbols binary is flagged "not stripped" (rustc is in the toolchain where this runs); the repo pins detector rejects a tree missing the strip profile.
  - `--repo`: the committed-tree pins — `[profile.release] strip` present in root Cargo.toml; the checklist doc exists; the source panic audit (panic!/expect!/unwrap! messages containing `.display()` or `{:?}` path interpolation) returns only reviewed-safe matches. Runs on every push (cheap, Linux-only step, no new job).
  - `--bin <path>`: the release-time binary gate — stripped verification (`file` output "stripped" on unix; RSDS/PDB-marker absence on windows), secret-prefix scan (`github_pat_`, `ghp_`, `gho_`, `AKIA`, `sk-`, `xox`, `ndus`, `browserid`, `csrfToken`, `jsToken`, `RCLONE_CONFIG_` values, PEM headers), build-machine path scan (`/home/`, `/root/`, `/Users/`, `C:\\Users\\`).
- **release.yml wiring**: after build, before collect — run `--bin target/release/IUM(.exe)` per-OS; a FAIL stops the release before any upload (nothing ships unaudited).
- **`docs/12_RE_Resistance_Checklist.md`** — the final flip checklist (amendment item 4): the four pins, each with mechanism + verification + current status; the compile-time-vs-runtime path distinction; the flip procedure (burn-any-leaked-credential rule, the private flip, post-flip CI-economics note).

## Gates

RED first: `--repo` run at HEAD fails on the missing strip profile + missing checklist doc (named reasons, evidence on this issue). Then: selftest PASS (all synthetic verdicts), repo mode GREEN, the full gate.sh on the committed tree, CI 8/8. The `--bin` mode's real verdict lands in release.yml at the next tag — same honesty contract as the app-e2e lane (capability-gated local skip, enforced at release).

## Out of scope (the amendment says so)

Aggressive obfuscation, name mangling, control-flow flattening. Native binaries stripped of symbols + no embedded secrets + no panic-path leakage clear an honest bar; obfuscation would break crash triage (the telemetry panic lane's whole point).
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": title, "body": body, "labels": []}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    it = json.loads(r.read().decode())
print("issue filed:", it["number"], it["html_url"])
