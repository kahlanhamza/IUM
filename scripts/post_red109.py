#!/usr/bin/env python3
"""Post the #109 RED-at-HEAD evidence."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/{path}",
        data=json.dumps(payload).encode() if payload else None,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


BODY = """## RED at HEAD — `ium-core/tests/dropbox_contract.rs`

The contract test pins (from the design note): `REDIRECT_PORT = 8738`,
`redirect_uri() == "http://localhost:8738"`, the official `content_hash`
algorithm against 5 vectors cross-checked with Python `hashlib` (empty,
1 MiB sub-block, exactly 4 MiB, 4 MiB + tail, 2 blocks + tail + 1), adapter
named `dropbox`, RESUMABLE-only capabilities, `uniform_block() == Some(4 MiB)`.

Failing output at current HEAD (no `dropbox` module exists):

```
$ cargo test -p ium-core --test dropbox_contract
error[E0432]: unresolved import `ium_core::dropbox`
  --> ium-core/tests/dropbox_contract.rs:19:15
19 | use ium_core::dropbox::{content_hash, redirect_uri, DropboxAdapter, REDIRECT_PORT};
   |               ^^^^^^^ could not find `dropbox` in `ium_core`
error: could not compile `ium-core` (test "dropbox_contract") due to 1 previous error
```

Implementation lands next; this target must compile and go green before any
push (gates non-bypassable). Also incorporates the rehearsal findings into the
contract: behind-offset append = silent no-op (at-least-once safety under the
same-bytes-per-offset invariant), ahead-offset = 409 realign, error shape
`{".tag": "incorrect_offset", "correct_offset": N}` siblings.
"""

r = api("POST", "issues/109/comments", {"body": BODY})
print("RED comment id:", r["id"])
