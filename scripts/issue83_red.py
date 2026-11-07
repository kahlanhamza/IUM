#!/usr/bin/env python3
"""RED evidence for #83 (WebDAV) — failing tests at HEAD (fda6c09), no implementation."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**RED** — failing tests at HEAD (fda6c09), before any implementation:

1. Core (`cargo test -p ium-core webdav`) — the tests exist, the constructors do not (E0425/E0599):

```
5 error[E0425]: cannot find function `normalize_vendor` in this scope
1 error[E0425]: cannot find value `WEBDAV_LANE` in this scope
2 error[E0599]: no associated function or constant named `webdav_config_from_parts` found for struct `adapter_rclone::RcloneBridgeConfig`
2 error[E0599]: no associated function or constant named `webdav_required` found for struct `adapter_rclone::RcloneBridgeConfig`
error: could not compile `ium-core` (lib test) due to 10 previous errors
```

2. Shell vitest (`src/logic.test.ts`) — the friendly-label table carries no webdav row:

```
34|     expect(friendlyAdapter("webdav")).toBe("WebDAV");
   |                                       ^
Test Files  1 failed (1)
Tests  1 failed | 12 passed (13)
```

3. Vault (`vault.rs :: webdav_lane_is_the_credential_sheet`) — asserts the schema carries the webdav lane
   (url/username/password required, password masked, vendor optional) and that `apply_env` maps the five
   `IUM_WEBDAV_*` names. The vault's pure-std harness needs the serde externs this sandbox cannot link
   (no WebKitGTK here — the gate skips the ium-app rust lane locally), so this test's failing run is
   owned by CI's `ium-app` job on every push; locally it is committed in the same commit as the fix it
   pins.

Baseline before this lane: 84 vitest + full cargo suite GREEN at fda6c09. Implementation follows.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/83/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted RED evidence: {out['html_url']}")
