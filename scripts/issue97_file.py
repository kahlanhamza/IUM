#!/usr/bin/env python3
"""File issue #97: the dependency-audit lane — nightly cargo-audit + the day-one catch."""
import json, urllib.request, pathlib

TOKEN = pathlib.Path(".secrets/gh_token").read_text().strip()

title = "The dependency-audit lane: nightly cargo-audit (pinned, detector-proven) + the day-one catch — libsqlite3-sys CVE"

body = """## Why this issue exists

The release-prep audit (2026-11-10) found no dependency-CVE scanning anywhere in the
workflows: SBOMs exist at release, but nothing *fails* when a CVE lands in the tree.
Big-tech cadence: automated vuln scanning with a loud gate. This lane adds the nightly
`cargo-audit` job — and its FIRST manual run caught a real vulnerability, before the
next release.

## The day-one catch (why this lane earned its existence immediately)

`cargo audit --file Cargo.lock` (cargo-audit 0.22.2, advisory DB 2026-11-10):

- **VULNERABILITY — RUSTSEC-2026-0459 (CVSS 6.8, medium)**: `libsqlite3-sys 0.30.1`
  bundles a C SQLite carrying **CVE-2026-119375**. Solution: `>= 0.39.0`.
  This is the SQLite underneath **the journal** — the engine's single source of truth.
  Not shippable in v0.2.1.
- 5 warnings (no gate failure, recorded here): `proc-macro-error 1.0.4` unmaintained
  (transitive), `zbus_names`/`zvariant`/`zvariant_derive 5.15.1` unmaintained (via the
  keyring's Linux secret-service path), `glib 0.18.5` unsound (via the GTK stack).
  Warnings stay warnings — the gate fails on vulnerabilities only.

## Design note

1. **The gate**: a nightly `audit` job in `nightly.yml` (+ workflow_dispatch). cargo-audit
   is a **pinned prebuilt musl binary** (v0.22.2) fetched with the #90 curl law and a
   digest pinned in the file (`sha256 7fb9497f…c428`, computed at adoption time over the
   TLS download — the rustsec release ships no checksum asset, so the digest-in-file IS
   the pin).
2. **The detector is proven to detect** (the R1 canary rule): the job first runs the tool
   against a committed hostile fixture lock (`scripts/audit/vuln_fixture.lock` —
   `chrono 0.4.19` → RUSTSEC-2020-0159) and requires a NON-ZERO exit; only then does it
   audit the real `Cargo.lock` and require exit 0. A detector that cannot detect is a no-op.
3. **The fix rides this issue**: bump `rusqlite` (0.32 → the newest minor that resolves
   `libsqlite3-sys >= 0.39`), `cargo update`, full gates — the journal suite
   (migrations, ENOSPC drills, soak bounds, the #96 upgrade pin against the v0.2.0
   fixture — now also proving the NEW sqlite opens the OLD journal's bytes) is the
   regression net. CHANGELOG [Unreleased] Fixed entry (user-visible: a CVE fix).

## Acceptance

- [ ] `libsqlite3-sys` resolves ≥ 0.39.0; `cargo audit` reports **0 vulnerabilities**.
- [ ] Nightly `audit` job: pinned tool (digest in file) → hostile-fixture canary (must
      fail) → real lock (must pass). Dispatch-proven.
- [ ] Gates green incl. the #96 upgrade pin (new sqlite opens the v0.2.0 journal).
- [ ] CHANGELOG entry for the CVE fix.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": title, "body": body, "labels": []}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
             "Content-Type": "application/json"},
    method="POST")
with urllib.request.urlopen(req) as r:
    d = json.load(r)
    print(f"filed #{d['number']}: {d['title']}")
    print(d["html_url"])
