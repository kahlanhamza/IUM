#!/usr/bin/env python3
"""File issue #89: nightly drill bootstrap must fail loud on upstream errors.

Nightly run 42468487548 (workflow run #12) failed both drill lanes in the
artifact bootstrap, not in the engine: the CDN answered 504 with an HTML body,
`curl -sSL --retry 3` (no --fail) saved that HTML into the checksum sidecar
with exit code 0, and the supply-chain guard then fired a cryptic
"sha256 mismatch (expected <html>...)". Guard held; triage cost a round-trip.
"""
import json
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """## Symptom (nightly run 42468487548 — workflow run #12, head 703e991)

Both drill lanes failed **in the artifact bootstrap**, before a single engine test ran:

- `tus_drill` — `[tus-test] FAIL: sha256 mismatch for tusd_linux_amd64.tar.gz (expected <html><body><h1>504 ... </body></html>, got bef140a1…)`
- `s3_drill` — `[s3-test] FAIL: md5 mismatch for linux_amd64.tar.gz (expected <html>…504 Gateway Time-out…, got dda6a9bf…)`

The engine code was never exercised. The `ci` workflow on the same commit was 8/8 GREEN (runs 42446590112, 42447183762).

## Root cause (wire facts)

`scripts/tus-test.sh` (L157–158) and `scripts/s3-test.sh` (L126–128, L136–138) fetch the
pinned artifact **and its checksum sidecar** with:

```
curl -sSL --retry 3 -o "$WORK/$ASSET.sha256" "$BASE_URL/$ASSET.sha256" || die "download …"
```

Missing `-f`/`--fail`: when the upstream CDN answers **HTTP 504 with an HTML body**, curl
exits **0** and writes the HTML error page into the sidecar file. The `|| die` never fires;
the supply-chain guard then compares the HTML against the real tarball hash and produces the
misleading "sha256 mismatch (expected `<html>…`)" — the guard held (failed closed, correct),
but the surface error sent triage looking for a checksum regression instead of a network
blip. Cost: one manual triage round-trip on a healthy tree.

Design note: `--retry 3` alone does not save us here — curl's transient-error retry set does
include 5xx, but without `--fail` the final 504 response still exits 0 and poisons the file.

## Fix (minimal, no scope creep)

1. **Fail loud on HTTP errors**: add `-f` to the eight artifact/sidecar fetches in both
   scripts (tarball, `.sha256` sidecar, `SHA256SUMS`, `.md5` sidecar) so any 4xx/5xx body
   never reaches a file and `|| die "download …"` fires with the true cause.
2. **Retry with backoff**: add `--retry-delay 2` beside the existing `--retry 3` so the
   transient-error window survives a CDN brown-out longer than back-to-back attempts.
3. **Sidecar format guard**: before comparing, validate the sidecar is actually a digest
   (hex, 32/64 chars, optionally followed by whitespace/filename). An HTML/JWT-looking
   sidecar dies with `sidecar is not a checksum — upstream served an error page`, naming
   the real failure even in the 200-with-error-page case (captive portal / proxy).

## Acceptance

- [ ] Local RED repro: a server that 504s the sidecar at HEAD produces the poisoned-file
      "mismatch" failure and exit 0 from curl; after the fix the same server produces
      `download … — upstream error` after the retries.
- [ ] Both scripts pass `bash -n` and the hardened fetch path is exercised by the local repro.
- [ ] `cargo test -p ium-core` stays green (no engine change).
- [ ] Nightly re-run of the failed jobs confirms the drills themselves pass.
- [ ] Gates + one commit + STATUS row + push; nightly runs clean on the next schedule.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps(
        {
            "title": "Drill bootstrap: fail loud on upstream HTTP errors + sidecar digest guard (nightly #12 flake)",
            "body": body,
            "labels": ["ci", "reliability"],
        }
    ).encode(),
    headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
    },
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    out = json.load(resp)
print(f"filed issue #{out['number']} url={out['html_url']}")
