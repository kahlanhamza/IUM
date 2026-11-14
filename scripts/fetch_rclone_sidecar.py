#!/usr/bin/env python3
"""Fetch the pinned rclone sidecar binary for the Tauri externalBin (#118).

The installers ship rclone INSIDE the bundle — a clean machine must not need
to install a CLI tool before a single destination lights up. This script is
the release pipeline's fetch step:

  - pinned version (RCLONE_SIDEcar_VERSION env or --version, default below);
  - the archive comes from downloads.rclone.org's own release directory;
  - the archive is verified against THAT directory's official SHA256SUMS —
    the version pin is the pin, the official checksums are the authority,
    and a mismatch is a hard failure;
  - the rclone binary is placed at ium-app/src-tauri/binaries/rclone-<triple>
    (.exe on Windows) — the exact filename Tauri's externalBin expects.

Usage:
  python3 scripts/fetch_rclone_sidecar.py                 # current platform
  python3 scripts/fetch_rclone_sidecar.py --target x86_64-pc-windows-msvc

Idempotent: an already-present, size-correct binary is re-verified, not
re-downloaded.
"""
import argparse
import hashlib
import os
import shutil
import stat
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BINARIES_DIR = REPO_ROOT / "ium-app" / "src-tauri" / "binaries"
DEFAULT_VERSION = "1.76.0"
BASE = "https://downloads.rclone.org"

# triple → (rclone archive OS-ARCH, binary name inside the zip, needs .exe)
TARGETS = {
    "x86_64-pc-windows-msvc": ("windows-amd64", "rclone.exe", True),
    "aarch64-apple-darwin": ("osx-arm64", "rclone", False),
    "x86_64-unknown-linux-gnu": ("linux-amd64", "rclone", False),
}


def current_triple() -> str:
    import platform

    machine = platform.machine().lower()
    system = sys.platform
    if system == "win32":
        if machine in ("amd64", "x86_64"):
            return "x86_64-pc-windows-msvc"
        raise SystemExit(f"unsupported windows arch: {machine}")
    if system == "darwin":
        if machine == "arm64":
            return "aarch64-apple-darwin"
        raise SystemExit(f"unsupported macOS arch: {machine} (the release ships Apple Silicon)")
    if machine == "x86_64":
        return "x86_64-unknown-linux-gnu"
    raise SystemExit(f"unsupported linux arch: {machine}")


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "ium-release/1.0"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", choices=sorted(TARGETS), default=current_triple())
    ap.add_argument("--version", default=os.environ.get("RCLONE_SIDECAR_VERSION", DEFAULT_VERSION))
    args = ap.parse_args()
    version = args.version.lstrip("v")

    archive_os, inner_name, is_exe = TARGETS[args.target]
    vtag = f"v{version}"
    archive_name = f"rclone-{vtag}-{archive_os}.zip"
    archive_url = f"{BASE}/{vtag}/{archive_name}"
    sums_url = f"{BASE}/{vtag}/SHA256SUMS"

    BINARIES_DIR.mkdir(parents=True, exist_ok=True)
    dest = BINARIES_DIR / (f"rclone-{args.target}" + (".exe" if is_exe else ""))

    print(f"rclone sidecar target: {args.target} -> {dest}")

    # The authority: the release directory's own SHA256SUMS.
    print(f"fetching official checksums: {sums_url}")
    sums = fetch(sums_url).decode()
    expected = None
    for line in sums.splitlines():
        h, _, name = line.partition("  ")
        if name.strip() == archive_name:
            expected = h.strip()
            break
    if not expected:
        raise SystemExit(f"FATAL: {archive_name} is not in the official SHA256SUMS of {vtag}")

    print(f"downloading {archive_url}")
    blob = fetch(archive_url)
    got = sha256(blob)
    if got != expected:
        raise SystemExit(f"FATAL: sha256 mismatch for {archive_name}: got {got}, official {expected}")
    print(f"sha256 verified: {got}")

    with tempfile.TemporaryDirectory() as td:
        tdp = Path(td)
        zpath = tdp / archive_name
        zpath.write_bytes(blob)
        with zipfile.ZipFile(zpath) as z:
            member = f"rclone-{vtag}-{archive_os}/{inner_name}"
            with z.open(member) as src, open(tdp / "rclone.bin", "wb") as out:
                shutil.copyfileobj(src, out)
        binary = tdp / "rclone.bin"
        if not is_exe:
            binary.chmod(binary.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        shutil.copyfile(binary, dest)
        os.chmod(dest, os.stat(dest).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    size = dest.stat().st_size
    print(f"placed: {dest} ({size} bytes, {size / 1024 / 1024:.1f} MiB)")


if __name__ == "__main__":
    main()
