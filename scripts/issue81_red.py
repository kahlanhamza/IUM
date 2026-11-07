#!/usr/bin/env python3
"""Post the #81 RED evidence comment."""
import json, urllib.request

TOK = open(".secrets/gh_token").read().strip()

core = open("/tmp/red_core.log").read()
shell = open("/tmp/red_shell.log").read()

def tail(txt, n=900):
    return txt[-n:]

BODY = f"""## RED — failing tests at HEAD (f665090), before implementation

Two failing seams, one per layer the lane touches. Full failing-test files are in the commit that follows.

**1. Core — `ium-core/tests/hot_registry.rs` (new):** the upsert seam does not exist. The supervisor hands the manager out as `Arc<JobManager>`; there is no `&mut` path back from a Tauri command, so the registry must take an interior-mutability upsert.

```
{tail(core)}
```

**2. Shell — `ium-app/src/auth.test.ts` (new):** the test-connection state machine (`./auth`) does not exist, and the enumerated copy carries no live-apply sentences (the restart sentence "…the next time IUM starts" still stands — that is the pain).

```
{tail(shell)}
```

The vault's SecretStore seam (keychain backing) is pinned by tests added inside `vault.rs` in the same commit — its compile proof rides CI per the #61 contract (local gates cover `ium-core` + the JS layers).

Next: minimal implementation (RwLock registry + `replace_adapter(&self)`; `SecretStore` with keyring-primary/file-fallback; `apply_credentials` + `probe_credentials` commands; the state machine + dialog copy), then full gates.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/81/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json", "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    out = json.load(r)
print("comment:", out["id"])
