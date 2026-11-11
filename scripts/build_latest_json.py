#!/usr/bin/env python3
"""Build `latest.json` — the tauri v2 updater manifest (#80).

The updater endpoint is `releases/latest/download/latest.json`; this script
turns the COLLECTED updater bundles + their minisign signatures into that
manifest. Rules enforced here, fail-loud:

- pairing: an updater bundle without its `.sig` sidecar is a shipped brick —
  the build refuses to declare it (no signature, no entry);
- one entry per platform, keyed by tauri's platform ids
  (darwin-aarch64, darwin-x86_64, linux-x86_64, windows-x86_64);
- the macOS bundle is `<productName>.app.tar.gz` — NO arch in the name — so
  its arch is DECLARED (`--darwin-arch`, the build leg's knowledge); an
  `.app.tar.gz` with no declared arch is a hard fail, never a filename guess
  (#100: the guess keyed the aarch64-only product as darwin-x86_64);
- v2 serves the INSTALLERS as the updater artifacts (verified in the v0.2.1
  release run logs): linux → the `.AppImage`, windows → the NSIS `.exe`
  (passive install), falling back to the `.msi`;
- URLs are absolute GitHub release-download links pointing at the PUBLIC
  distro repo `kahlanhamza/IUM-dist` (#101: the source repo is closed — a
  private repo's release assets 404 for every user, the distro repo is the
  distribution surface);
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
    ("linux-x86_64", ".AppImage", False),
    ("windows-x86_64", ".exe", True),   # NSIS setup — passive install
    ("windows-x86_64", ".msi", False),
]


def parse_name(name: str, darwin_arch: str) -> tuple[str, str] | None:
    """Map an updater artifact to (platform, filename) by its v2 shape.

    Tauri v2 (`createUpdaterArtifacts`) signs the installers themselves and
    wraps only macOS in .app.tar.gz (run-42816215486 wire fact). The macOS
    wrapper carries no arch in its name — the caller DECLARES it.
    """
    n = name.lower()
    if n.endswith(".app.tar.gz"):
        if "aarch64" in n or "arm64" in n:
            return ("darwin-aarch64", name)
        if "x86_64" in n or "x64" in n or "intel" in n:
            return ("darwin-x86_64", name)
        # no arch in the name: the DECLARED arch decides — none declared is
        # the caller's bug and fails loud in build() (#100)
        if not darwin_arch:
            return ("darwin-UNDECLARED", name)
        return (f"darwin-{darwin_arch}", name)
    if n.endswith(".appimage"):
        return ("linux-x86_64", name)
    if n.endswith(".exe"):
        return ("windows-x86_64", name)
    if n.endswith(".msi"):
        return ("windows-x86_64", name)
    return None


def build(dist_dir: pathlib.Path, version: str, notes: str, pub_date: str,
          repo_slug: str, tag: str, darwin_arch: str = "aarch64") -> dict:
    dist_dir = pathlib.Path(dist_dir)
    sigs = {p.name[: -len(".sig")]: p for p in dist_dir.glob("*.sig")}
    platforms: dict[str, dict] = {}
    windows_seen: dict[str, str] = {}
    for bundle in sorted(dist_dir.glob("*")):
        if bundle.name.endswith(".sig") or bundle.name.endswith(".json"):
            continue
        parsed = parse_name(bundle.name, darwin_arch)
        if parsed is None:
            continue
        platform = parsed[0]
        if platform.startswith("darwin-UNDECLARED"):
            raise SystemExit(
                f"UNKEYED FAIL: {bundle.name} is a macOS updater bundle with no arch in "
                "its name and none declared via --darwin-arch — refusing to guess "
                "(#100: the guess shipped darwin-x86_64 for an aarch64-only product, "
                "a key no shipped hardware reports)."
            )
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
            # both the NSIS .exe and the .msi may exist; the NSIS setup wins
            # (installMode: passive in tauri.conf.json)
            windows_seen[platform] = windows_seen.get(platform)
            if bundle.name.endswith(".exe") or platform not in windows_seen:
                windows_seen[platform] = entry
            if bundle.name.endswith(".exe"):
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
    """Synthetic dist tree → proves pairing, platform mapping, NSIS preference.

    The fixture uses the artifacts Tauri v2 ACTUALLY emits (verified in the
    v0.2.1 release run 42816215486 logs): `createUpdaterArtifacts` signs the
    INSTALLERS themselves (.deb.sig / .AppImage.sig / .rpm.sig / .exe.sig /
    .msi.sig) and wraps only macOS in .app.tar.gz — the v1 wrapper bundles
    (.AppImage.tar.gz, .nsis.zip, .msi.zip) never occur on v2 and naming them
    here tested a fiction that masked #100.
    """
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        d = pathlib.Path(td)
        sig = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHNpZw"
        files = {
            "IUM_0.2.1_amd64.deb": sig,
            "IUM_0.2.1_amd64.deb.sig": sig,
            "IUM_0.2.1_amd64.AppImage": sig,
            "IUM_0.2.1_amd64.AppImage.sig": sig,
            "IUM_0.2.1_x64-setup.exe": sig,
            "IUM_0.2.1_x64-setup.exe.sig": sig,
            "IUM_0.2.1_x64_en-US.msi": sig,
            "IUM_0.2.1_x64_en-US.msi.sig": sig,
            "IUM.app.tar.gz": sig,
            "IUM.app.tar.gz.sig": sig,
        }
        for name, s in files.items():
            if name.endswith(".sig"):
                (d / name).write_text(s + "\n")
            else:
                (d / name).write_bytes(b"bundle")
        # The macOS updater bundle carries NO arch in its name (the product
        # names it <productName>.app.tar.gz) — the arch is the build leg's
        # knowledge and MUST be passed, never guessed from the filename
        # (#100: the shape heuristic keyed the aarch64-only product as
        # darwin-x86_64, a key no shipped hardware reports).
        manifest = build(d, "0.2.1", "Fixes and polish.", "2026-11-10T00:00:00Z",
                         "kahlanhamza/IUM-dist", "v0.2.1", darwin_arch="aarch64")
        assert set(manifest["platforms"]) == {
            "darwin-aarch64", "linux-x86_64", "windows-x86_64"
        }, manifest["platforms"].keys()
        assert manifest["platforms"]["darwin-aarch64"]["url"].endswith(
            "IUM.app.tar.gz"), "the macOS bundle keys under the PASSED arch"
        assert manifest["platforms"]["linux-x86_64"]["url"].endswith(
            "IUM_0.2.1_amd64.AppImage"), "v2 serves the AppImage directly"
        assert manifest["platforms"]["windows-x86_64"]["url"].endswith(
            "IUM_0.2.1_x64-setup.exe"), "NSIS setup wins on windows (passive)"
        assert all(p["signature"] == sig for p in manifest["platforms"].values())
        assert all(
            p["url"].startswith(
                "https://github.com/kahlanhamza/IUM-dist/releases/download/"
            )
            for p in manifest["platforms"].values()
        ), "manifest asset URLs are the PUBLIC distro repo's (#101)"
        assert manifest["version"] == "0.2.1" and manifest["pub_date"] == "2026-11-10T00:00:00Z"
        # the pairing rule: an unsigned bundle must FAIL the build
        (d / "IUM_0.2.3_x64-setup.exe").write_bytes(b"bundle")
        try:
            build(d, "0.2.3", "", "2026-11-10T00:00:00Z", "kahlanhamza/IUM-dist",
                  "v0.2.3", darwin_arch="aarch64")
        except SystemExit as e:
            assert "PAIRING FAIL" in str(e)
        else:
            raise AssertionError("unsigned bundle did not fail the build")
        # an updater bundle whose arch was never declared cannot be keyed —
        # refusing is the honest answer, a wrong key is #100
        (d / "SomeOther.app.tar.gz").write_bytes(b"bundle")
        (d / "SomeOther.app.tar.gz.sig").write_text(sig + "\n")
        try:
            build(d, "0.2.3", "", "2026-11-10T00:00:00Z", "kahlanhamza/IUM",
                  "v0.2.3", darwin_arch="")
        except SystemExit as e:
            assert "UNKEYED" in str(e), str(e)
        else:
            raise AssertionError("an undeclared .app.tar.gz did not fail the build")
    print("SELF-TEST PASS: v2 pairing, platform map, NSIS preference, schema")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dist", help="dist_release dir with updater bundles + .sig files")
    ap.add_argument("--version", help="release version, e.g. 0.2.0")
    ap.add_argument("--notes", default="See the release notes on GitHub.")
    ap.add_argument("--pub-date", help="ISO 8601 timestamp, e.g. 2026-11-08T00:00:00Z")
    ap.add_argument("--repo", default="kahlanhamza/IUM-dist",
                    help="the release repo the asset URLs point at — the PUBLIC "
                         "distro repo by default (#101; the source repo is "
                         "private, its assets 404 for users)")
    ap.add_argument("--tag", help="release tag, e.g. v0.2.0")
    ap.add_argument("--darwin-arch", default="aarch64",
                    choices=["aarch64", "x86_64"],
                    help="arch of the macOS build leg — the .app.tar.gz name "
                         "carries none, so the leg declares it (#100); the "
                         "product ships Apple Silicon only, hence the default")
    ap.add_argument("--out", help="where to write latest.json")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not (a.dist and a.version and a.tag and a.out):
        ap.error("--dist/--version/--tag/--out are required (or run --self-test)")
    manifest = build(pathlib.Path(a.dist), a.version, a.notes,
                     a.pub_date or "", a.repo, a.tag, darwin_arch=a.darwin_arch)
    out = pathlib.Path(a.out)
    out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"latest.json written: {out} ({len(manifest['platforms'])} platforms)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
