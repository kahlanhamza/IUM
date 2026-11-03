#!/usr/bin/env python3
"""Pack the IUM repo into a Kaggle dataset (the kernel's source input).

Why a dataset and not a git clone: the repo is PRIVATE, and cloning from
Kaggle would need the GitHub PAT inside the kernel — a credential we do
not paste into an external service. The repo tarball rides as a private
dataset under the owner's account instead; secrets stay in .secrets/.

Excludes build output, node_modules, toolchains and secrets. Safe to
re-run: creates the dataset the first time and versions it afterwards.

Usage: python3 scripts/kaggle/prepare_source_dataset.py
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STAGE = ROOT / ".kaggle-stage" / "ium-source"


def main() -> int:
    print(f"[kaggle-dataset] packing working tree of {ROOT}")
    STAGE.mkdir(parents=True, exist_ok=True)
    tar = STAGE / "ium-source.tar.gz"
    if tar.exists():
        tar.unlink()
    subprocess.run(
        ["tar", "--exclude=.git", "--exclude=target",
         "--exclude=node_modules", "--exclude=.secrets",
         "--exclude=.toolchain", "--exclude=rea-lab",
         "--exclude=tool-results", "--exclude=.kaggle-stage",
         "--exclude=dist", "--exclude=__pycache__",
         "--exclude=*.log", "--exclude=.kaggle-stage",
         "-czf", str(tar), "."],
        cwd=ROOT, check=True,
    )
    print(f"[kaggle-dataset] tarball: {tar.stat().st_size/1e6:.1f} MB")

    meta = {
        "title": "ium-source",
        "id": "hamzakhl/ium-source",
        "licenses": [{"name": "MIT"}],
    }
    (STAGE / "dataset-metadata.json").write_text(json.dumps(meta, indent=2))
    r = subprocess.run(["kaggle", "datasets", "create", "-p", str(STAGE)],
                       capture_output=True, text=True)
    out = (r.stdout + r.stderr).strip()
    print(out)
    if r.returncode == 0 and "error" not in out.lower():
        print("[kaggle-dataset] dataset created (hamzakhl/ium-source)")
        return 0
    # slug exists (or create failed) → push a new VERSION in place
    r2 = subprocess.run(
        ["kaggle", "datasets", "version", "-p", str(STAGE),
         "-m", "source sync", "--dir-mode", "zip"],
        capture_output=True, text=True)
    print((r2.stdout + r2.stderr).strip())
    if r2.returncode == 0:
        print("[kaggle-dataset] dataset versioned (hamzakhl/ium-source)")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
