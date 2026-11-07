#!/usr/bin/env python3
"""RED evidence for #82 — the tests exist, the modules do not."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**RED** — both surfaces measured before any implementation exists:

1. The vitest surface (`src/connect.test.ts` — the connect-state contracts + the OAuth backend list):

```
FAIL src/connect.test.ts — Failed to resolve import "./connect"
  code: 'ERR_MODULE_NOT_FOUND'
```

2. The rust surface (the `rustc --test` harness, the #61 contract — pure-std, zero dependencies):

```
$ rustc --test ium-app/src-tauri/src/auth.rs -o /tmp/auth82
error: couldn't find file `src-tauri/src/auth.rs`
error: aborting due to 1 previous error
```

Baseline before this lane: 84/84 vitest GREEN. Implementation follows.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/82/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted RED evidence: {out['html_url']}")
