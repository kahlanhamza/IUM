#!/usr/bin/env python3
"""Build `latest.json` — the tauri v2 updater manifest (#80).

The updater endpoint is `releases/latest/download/latest.json`; this script
turns the COLLECTED updater bundles + their minisign signatures into that
manifest. Rules enforced here, fail-loud:

- pairing: an updater bundle without its `.sig` sidecar is a shipped brick —
  the build refuses to declare it (no signature, no entry);
- one entry per platform, keyed by tauri's platform ids
  (darwin-aarch64, darwin-x86_64, linux-x86_64, windows-x86_64);
- Windows prefers the NSIS bundle (installMode: passive in tauri.conf.json);
- URLs are absolute GitHub release-download links (the endpoint serves the
  manifest, the assets carry the bytes);
- `--self-test` proves all of the above on a synthetic tree (the house
  pattern: a gate that cannot rot silently).

stdlib only — runs in the release job next to the collectors.
"""
import argparse
import json
import pathlib
import sys

PLATFORM_RULES = [
    # (platform id, filename suffix of the updater bundle, prefer-when-present)
    ("darwin-aarch64", ".app.tar.gz", False),
    ("darwin-x86_64", ".app.tar.gz", False),
    ("linux-x86_64", ".AppImage.tar.gz", False),
    ("windows-x86_64", ".nsis.zip", True),
    ("windows-x86_64", ".msi.zip", False),
]


def parse_name(name: str) -> tuple[str, str] | None:
    """Map an updater bundle filename to (platform, arch) by its shape."""
    n = name.lower()
    if n.endswith(".app.tar.gz"):
        return ("darwin-aarch64", name) if "aarch64" in n else ("darwin-x86_64", name)
    if n.endswith(".appimage.tar.gz"):
        return ("linux-x86_64", name)
    if n.endswith(".nsis.zip") or n.endswith(".msi.zip"):
        return ("windows-x86_64", name)
    return None


def build(dist_dir: pathlib.Path, version: str, notes: str, pub_date: str,
          repo_slug: str, tag: str) -> dict:
    dist_dir = pathlib.Path(dist_dir)
    sigs = {p.name[: -len(".sig")]: p for p in dist_dir.glob("*.sig")}
    platforms: dict[str, dict] = {}
    windows_seen: dict[str, str] = {}
    for bundle in sorted(dist_dir.glob("*")):
        if bundle.name.endswith(".sig") or bundle.name.endswith(".json"):
            continue
        parsed = parse_name(bundle.name)
        if parsed is None:
            continue
        _, _ = parsed
        platform = parsed[0]
        if bundle.name not in sigs:
            raise SystemExit(
                f"PAIRING FAIL: {bundle.name} has no .sig sidecar — refusing to declare "
                "an unsigned updater entry (a manifest entry without a signature is a "
                "shipped brick: the client rejects it at install time)."
            )
        entry = {
            "signature": sigs[bundle.name].read_text(encoding="utf-8").strip(),
            "url": f"https://github.com/{repo_slug}/releases/download/{tag}/{bundle.name}",
        }
        if platform == "windows-x86_64":
            # both .nsis.zip and .msi.zip may exist; NSIS wins (passive mode)
            windows_seen[platform] = windows_seen.get(platform)
            if bundle.name.endswith(".nsis.zip") or platform not in windows_seen:
                windows_seen[platform] = entry
            if bundle.name.endswith(".nsis.zip"):
                windows_seen["__prefer__"] = entry
        else:
            platforms[platform] = entry
    if "__prefer__" in windows_seen:
        platforms["windows-x86_64"] = windows_seen["__prefer__"]
    elif windows_seen.get("windows-x86_64"):
        platforms["windows-x86_64"] = windows_seen["windows-x86_64"]
    return {
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": platforms,
    }


def self_test() -> int:
    """Synthetic dist tree → proves pairing, platform mapping, NSIS preference."""
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        d = pathlib.Path(td)
        sig = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHNpZw"
        files = {
            "IUM_0.2.0_aarch64.app.tar.gz": sig,
            "IUM_0.2.0_x64.app.tar.gz": sig,
            "IUM_0.2.0_amd64.AppImage.tar.gz": sig,
            "IUM_0.2.0_x64-setup.nsis.zip": sig,
            "IUM_0.2.0_x64_en-US.msi.zip": sig,
        }
        for name, s in files.items():
            (d / name).write_bytes(b"bundle")
            (d / (name + ".sig")).write_text(s + "\n")
        manifest = build(d, "0.2.0", "Fixes and polish.", "2026-11-08T00:00:00Z",
                         "kahlanhamza/IUM", "v0.2.0")
        assert set(manifest["platforms"]) == {
            "darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"
        }, manifest["platforms"].keys()
        assert manifest["platforms"]["windows-x86_64"]["url"].endswith(
            "IUM_0.2.0_x64-setup.nsis.zip"), "NSIS must win on windows"
        assert manifest["platforms"]["darwin-aarch64"]["url"].endswith("aarch64.app.tar.gz")
        assert all(p["signature"] == sig for p in manifest["platforms"].values())
        assert manifest["version"] == "0.2.0" and manifest["pub_date"] == "2026-11-08T00:00:00Z"
        # the pairing rule: an unsigned bundle must FAIL the build
        (d / "IUM_0.2.1_x64-setup.nsis.zip").write_bytes(b"bundle")
        try:
            build(d, "0.2.1", "", "2026-11-08T00:00:00Z", "kahlanhamza/IUM", "v0.2.1")
        except SystemExit as e:
            assert "PAIRING FAIL" in str(e)
        else:
            raise AssertionError("unsigned bundle did not fail the build")
    print("SELF-TEST PASS: pairing, platform map, NSIS preference, schema")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dist", help="dist_release dir with updater bundles + .sig files")
    ap.add_argument("--version", help="release version, e.g. 0.2.0")
    ap.add_argument("--notes", default="See the release notes on GitHub.")
    ap.add_argument("--pub-date", help="ISO 8601 timestamp, e.g. 2026-11-08T00:00:00Z")
    ap.add_argument("--repo", default="kahlanhamza/IUM")
    ap.add_argument("--tag", help="release tag, e.g. v0.2.0")
    ap.add_argument("--out", help="where to write latest.json")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not (a.dist and a.version and a.tag and a.out):
        ap.error("--dist/--version/--tag/--out are required (or run --self-test)")
    manifest = build(pathlib.Path(a.dist), a.version, a.notes,
                     a.pub_date or "", a.repo, a.tag)
    out = pathlib.Path(a.out)
    out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"latest.json written: {out} ({len(manifest['platforms'])} platforms)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
