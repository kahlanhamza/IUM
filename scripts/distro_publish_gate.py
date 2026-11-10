#!/usr/bin/env python3
"""The distro-publish hygiene gate (#102): nothing internal ships to the public
distro repo. Parses .github/workflows/release.yml + ium-app/src-tauri/tauri.conf.json
and asserts the publish surface is the public set ONLY.

Checks (D6):
  (a) SBOM outputs target release_provenance/ (never release_assets/)
  (b) attestation bundle copies target release_provenance/ (never release_assets/)
  (c) the release upload references only release_assets/*
  (d) the notes generator carries no internal issue refs and no cosign talk
  (e) the baked updater endpoint is the public distro feed

Exit 0 = clean, 1 = leak-shaped. --self-test runs the embedded verdict fixtures.
"""
import json
import re
import sys

WF = ".github/workflows/release.yml"
CONF = "ium-app/src-tauri/tauri.conf.json"
DISTRO = "kahlanhamza/IUM-dist"


def load(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read()


def run_checks(wf_text, conf_text):
    """Return list of (ok, label, detail)."""
    checks = []

    # (a) SBOM outputs must target release_provenance/
    sbom_ok = (
        "release_provenance/IUM-${TAG}-rust.cdx.json" in wf_text
        and "release_provenance/IUM-${TAG}-app.cdx.json" in wf_text
        and not re.search(r"-o\s+\"?cyclonedx-json=release_assets/", wf_text)
        and not re.search(r">>\s*\"?release_assets/.*cdx\.json", wf_text)
    )
    checks.append((sbom_ok, "(a) SBOM outputs -> release_provenance/",
                   "syft/npm SBOM writes must stay out of the public set"))

    # (b) attestation bundles must be copied to release_provenance/
    bundle_ok = (
        re.search(r"cp \"\$j\" \"release_provenance/", wf_text) is not None
        and re.search(r"cp \"\$j\" \"release_assets/", wf_text) is None
    )
    checks.append((bundle_ok, "(b) attestation bundles -> release_provenance/",
                   "DSSE provenance envelopes name the private repo"))

    # (c) the release upload references only release_assets/*
    up = re.findall(r"gh release (?:create|upload)[^\n]*", wf_text)
    up_ok = bool(up) and all(
        ("release_assets" in line or "-R" in line or "--" in line or "release_notes" in line)
        for line in up
    ) and all("release_provenance" not in line for line in up)
    checks.append((up_ok, "(c) the release upload touches release_assets/ only",
                   " | ".join(l.strip()[:60] for l in up) or "no upload lines found"))

    # (d) the notes generator: no internal issue refs, no cosign talk
    notes = re.search(r"\{\s*echo \"## The installers\".*?\} > \.\./notes_body\.md",
                      wf_text, re.S)
    if notes is None:
        checks.append((False, "(d) the notes generator is user-facing",
                       "the notes block was not found"))
    else:
        body = notes.group(0)
        bad_refs = re.findall(r"#\d+", body)
        bad_words = [w for w in ("cosign", "SBOM", "attestation", "sigstore") if w in body]
        checks.append((not bad_refs and not bad_words,
                       "(d) the notes generator is user-facing",
                       (f"internal refs {bad_refs} " if bad_refs else "")
                       + (f"internal words {bad_words}" if bad_words else "clean")))

    # (e) the baked endpoint is the distro feed
    conf = json.loads(conf_text)
    eps = conf["plugins"]["updater"]["endpoints"]
    ep_ok = eps == [f"https://github.com/{DISTRO}/releases/latest/download/latest.json"]
    checks.append((ep_ok, "(e) the baked updater endpoint is the distro feed",
                   str(eps)))

    return checks


def self_test():
    """Fixture verdicts: a leak-shaped workflow must fail, the clean shape must pass."""
    leaky = (
        '-o "cyclonedx-json=release_assets/IUM-${TAG}-rust.cdx.json"\n'
        '> "release_assets/IUM-${TAG}-app.cdx.json"\n'
        'cp "$j" "release_assets/$(basename "$f").attestation.$k.jsonl"\n'
        'gh release create "$TAG" -R "$DISTRO_REPO" release_assets/*\n'
        '{ echo "## The installers"; echo "cosign verify-blob-attestation (see #65 #71 #101)"; } > ../notes_body.md\n'
    )
    clean = (
        '-o "cyclonedx-json=release_provenance/IUM-${TAG}-rust.cdx.json"\n'
        '> "release_provenance/IUM-${TAG}-app.cdx.json"\n'
        'cp "$j" "release_provenance/$(basename "$f").attestation.$k.jsonl"\n'
        'gh release create "$TAG" -R "$DISTRO_REPO" release_assets/*\n'
        '{ echo "## The installers"; echo "sha256sum -c SHA256SUMS.txt"; } > ../notes_body.md\n'
    )
    conf = json.dumps({"plugins": {"updater": {"endpoints": [
        f"https://github.com/{DISTRO}/releases/latest/download/latest.json"]}}})

    leak_results = run_checks(leaky, conf)
    clean_results = run_checks(clean, conf)
    leak_fails = sum(1 for ok, _, _ in leak_results if not ok)
    clean_fails = sum(1 for ok, _, _ in clean_results if not ok)
    print(f"SELF-TEST: leaky fixture -> {leak_fails} failing checks (expect >=3)")
    print(f"SELF-TEST: clean fixture -> {clean_fails} failing checks (expect 0)")
    if leak_fails < 3 or clean_fails != 0:
        print("SELF-TEST: FAIL — the gate does not discriminate")
        return 1
    print("SELF-TEST: PASS — the gate detects the leak shape and passes the clean shape")
    return 0


def main():
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    checks = run_checks(load(WF), load(CONF))
    failed = 0
    for ok, label, detail in checks:
        print(("  OK  " if ok else "  FAIL"), label, ("— " + detail if detail and not ok else ""))
        failed += 0 if ok else 1
    print("DISTRO-PUBLISH GATE:", "PASS" if failed == 0 else f"FAIL ({failed})")
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
