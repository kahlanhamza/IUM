#!/usr/bin/env python3
"""RED evidence for #85 (YouTube) — failing tests at HEAD (closer of #84), no implementation."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()

BODY = """**RED** — the lane's contract suite written BEFORE any implementation:

1. Core (`cargo test -p ium-core --test youtube_contracts`) — the modules do not exist (E0432):

```
error[E0432]: unresolved import `ium_core::google`
error[E0425]: unresolved import `ium_core::adapter_youtube`
error: could not compile `ium-core` (test "youtube_contracts") due to 2 previous errors
```

The suite pins, test by test: the RFC 7636 Appendix B PKCE vector, the consent-URL shape
(`scope=youtube.upload`, `access_type=offline`, `redirect_uri=http://localhost:8737`, S256), the
percent-decoded loopback-redirect parse + the CSRF state-mismatch refusal, TokenSet parse/expiry
(60 s skew)/roundtrip, the resumable metadata body (privacyStatus **private** by default), the
Content-Range wire language, the 308-Range + final-body video-id parse, the 100-char title
sanitizer, the honest capability declaration (8 MiB chunks = 256 KiB multiples, max_parallel 1 —
one session is ONE sequential byte stream), and the loud missing-credential refusals naming the
env var.

2. Vault (`vault.rs :: youtube_lane_is_two_fields_and_a_hidden_token`) — CI's `ium-app` job owns the
   failing run (the sandbox cannot link the serde externs — the standing #61 note).

3. Shell — the loopback sign-in command (`connect_youtube`) does not exist; its compile proof is CI's
   ium-app job on all 3 OS.

Implementation follows: `ium-core/src/google.rs` (the OAuth pure math + the reqwest exchange/refresh),
`ium-core/src/adapter_youtube.rs` (the resumable adapter), the loopback redirect server in the shell,
the vault lane, and the sheet with the two one-time client fields + one Sign-in button.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/85/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted RED evidence: {out['html_url']}")
