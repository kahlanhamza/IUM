#!/usr/bin/env python3
"""Installer-size budget gate (product bar #66 lane 5, issue #72).

The #65 bar says installers are "lightweight + fast, MEASURED" — the
measurement shipped in v0.1.1, the enforcement lives here: an installer that
exceeds its budget FAILS the release job before the publish step can ship
it. Growth is a reviewed diff to ``installer_budget.json``, never a silent
ship.

Rules (no silent passes):
  * every installer-suffix file found in the scanned dir must have a budget
    entry (suffix → budget); an UNKNOWN installer suffix is a failure — a
    new bundle format without a decided budget does not ship;
  * a file over ``max_bytes`` is a failure (the table shows actual vs
    budget vs baseline and the headroom left);
  * non-installer files (sha256 sidecars, SBOMs, sums, attestations) are
    ignored — they are not the product's ship size.

Usage:
  python3 installer_size_gate.py --dir ium-app/dist_release
  python3 installer_size_gate.py --self-test      # synthetic verdict proof

stdlib only (runners have python3; no pip step in the release path).
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path

MIB = 1024 * 1024

# Installer suffixes the gate budgets (lowercase match; .exe = the NSIS
# setup — Tauri's only .exe artifact on Windows). The FAMILY list is
# deliberately wider than the budget table: a NEW bundle format (.snap,
# .pkg, .apk...) hits "no budget entry" and FAILS — a decided budget is
# part of shipping the format, not an afterthought.
INSTALLER_SUFFIXES = (
    ".msi", ".exe", ".dmg", ".deb", ".appimage", ".rpm",
    ".snap", ".flatpak", ".pkg", ".appx", ".apk",
)

EXIT_OK = 0
EXIT_VIOLATION = 1
EXIT_USAGE = 2


def budget_for(budgets: dict, suffix: str) -> dict | None:
    return budgets.get(suffix)


def check_dir(artifacts_dir: Path, budgets: dict) -> tuple[list[str], bool]:
    """Scan ``artifacts_dir``; return (report lines, all_ok)."""
    lines: list[str] = []
    ok = True
    files = sorted(p for p in artifacts_dir.iterdir() if p.is_file())
    installers = [
        p for p in files if p.suffix.lower() in INSTALLER_SUFFIXES
    ]
    if not installers:
        return [f"GATE-FAIL: no installer artifacts found in {artifacts_dir}"], False

    header = (
        f"{'artifact':<40} {'actual':>10} {'budget':>10} {'baseline':>10} "
        f"{'headroom':>9}  verdict"
    )
    lines.append(header)
    lines.append("-" * len(header))

    for path in installers:
        actual = path.stat().st_size
        entry = budget_for(budgets, path.suffix.lower())
        if entry is None:
            ok = False
            lines.append(
                f"{path.name:<40} {actual:>10,} {'—':>10} {'—':>10} "
                f"{'—':>9}  FAIL: no budget entry for '{path.suffix.lower()}' "
                f"— decide one in installer_budget.json"
            )
            continue
        budget = entry["max_bytes"]
        baseline = entry.get("baseline_bytes")
        verdict = "OK" if actual <= budget else "FAIL: over budget"
        ok = ok and actual <= budget
        headroom = f"{(budget - actual) / MIB:.1f} MB" if actual <= budget else "-0 MB"
        base_disp = (
            f"{baseline / MIB:.2f}" if isinstance(baseline, int) else "—"
        )
        lines.append(
            f"{path.name:<40} {actual / MIB:>9.2f}M {budget / MIB:>9.2f}M "
            f"{base_disp:>9}M {headroom:>9}  {verdict} "
            f"[{entry.get('name', path.suffix)}]"
        )

    ignored = [p.name for p in files if p not in installers]
    if ignored:
        lines.append(f"(not budgeted, not the ship size: {', '.join(ignored)})")
    return lines, ok


def self_test() -> int:
    """Prove all four verdicts on synthetic fixtures with a tiny budget
    table (no release needed to exercise the gate logic)."""
    budgets = {
        ".msi": {"name": "t", "baseline_bytes": 1 * MIB, "max_bytes": 2 * MIB},
        ".deb": {"name": "t", "baseline_bytes": 1 * MIB, "max_bytes": 2 * MIB},
    }

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)

        # 1. under budget → OK
        d1 = root / "ok"
        d1.mkdir()
        (d1 / "IUM_0.1.1_x64_en-US.msi").write_bytes(b"x" * MIB)
        lines, ok = check_dir(d1, budgets)
        assert ok, f"under-budget fixture must pass: {lines}"

        # 2. over budget → FAIL
        d2 = root / "over"
        d2.mkdir()
        (d2 / "IUM_0.1.1_amd64.deb").write_bytes(b"x" * (3 * MIB))
        lines, ok = check_dir(d2, budgets)
        assert not ok, f"over-budget fixture must fail: {lines}"
        assert any("over budget" in l for l in lines), lines

        # 3. unknown installer suffix → FAIL (no silent pass)
        d3 = root / "unknown"
        d3.mkdir()
        (d3 / "ium_0.1.1.apk").write_bytes(b"x" * MIB)
        lines, ok = check_dir(d3, budgets)
        assert not ok, f"unknown-suffix fixture must fail: {lines}"
        assert any("no budget entry" in l for l in lines), lines

        # 4. sidecars / SBOMs are not installers → ignored, no false alarm
        d4 = root / "sidecars"
        d4.mkdir()
        (d4 / "IUM_0.1.1_x64_en-US.msi").write_bytes(b"x" * MIB)
        (d4 / "IUM_0.1.1_x64_en-US.msi.sha256").write_text("ab " * 32 + "\n")
        (d4 / "ium-0.1.1.cdx.json").write_text("{}")
        (d4 / "SHA256SUMS.txt").write_text("")
        lines, ok = check_dir(d4, budgets)
        assert ok, f"sidecar fixture must pass: {lines}"

        # 5. empty dir → loud failure, not a silent pass
        d5 = root / "empty"
        d5.mkdir()
        lines, ok = check_dir(d5, budgets)
        assert not ok, "an empty artifacts dir must fail loudly"

    print("GATE-SELFTEST PASS: ok / over-budget / unknown-suffix / sidecar-ignored / empty-dir-loud")
    return EXIT_OK


def main() -> int:
    here = Path(__file__).resolve().parent
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dir", type=Path, default=Path.cwd(),
                    help="directory holding the collected installer artifacts")
    ap.add_argument("--budget", type=Path, default=here / "installer_budget.json",
                    help="path to installer_budget.json")
    ap.add_argument("--self-test", action="store_true",
                    help="run the synthetic verdict proof and exit")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    budgets = json.loads(args.budget.read_text())["budgets"]
    lines, ok = check_dir(args.dir, budgets)
    for line in lines:
        print(line)
    if ok:
        print("INSTALLER-SIZE-GATE PASS")
        return EXIT_OK
    print("INSTALLER-SIZE-GATE FAIL — an installer over budget or without a "
          "decided budget does not ship (#66 lane 5)")
    return EXIT_VIOLATION


if __name__ == "__main__":
    sys.exit(main())
