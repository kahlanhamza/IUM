#!/usr/bin/env python3
"""RED evidence for #84 (Google Drive) — failing tests at HEAD (closer of #83), no implementation."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**RED** — failing tests at HEAD, before any implementation:

1. The rustc `--test` harness (the #61 contract) — the FLIPPED allowlist test run against the HEAD implementation:

```
test tests::the_allowlist_is_the_obvious_oauth_ones_drive_included ... FAILED
thread panicked at /tmp/auth_red.rs:126:13:
assertion failed: is_oauth_backend(id)
```

2. Core (`cargo test -p ium-core google_drive`) — the tests exist, the lane does not (E0425/E0599):

```
error[E0425]: cannot find value `GOOGLE_DRIVE_LANE` in this scope
error[E0599]: no associated function or constant named `google_drive_config_from_parts` found
error[E0599]: no associated function or constant named `google_drive_required` found
error: could not compile `ium-core` (lib test) due to 4 previous errors
```

3. Shell vitest — the allowlist still pins the four-backend list, and `hidden` does not travel:

```
FAIL src/connect.test.ts — 3 failed (five-ids, drive-allowlisted, injection-guard expects drive=true)
FAIL src/settings.test.ts — toRows carries the hidden flag from the view (token.hidden is undefined)
Tests  4 failed | 17 passed (21)
```

4. Vault (`vault.rs :: google_drive_lane_is_one_hidden_token_row`) — asserts the schema carries exactly one
   hidden required secret `token` and the apply_env mapping. CI's `ium-app` job owns the failing run (the
   sandbox cannot link the serde externs — the standing #61 note).

The owner's ruling on record: *"Go ahead with the WebDAV, also with Google drive and YouTube, if it is
possible for user to upload just by connecting his accounts."* Implementation follows.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/84/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted RED evidence: {out['html_url']}")
