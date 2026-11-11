#!/usr/bin/env python3
"""Post the RED evidence for #105 (guided region never reaches the scope)."""
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()

BODY = """## RED evidence (at HEAD `100dfbf`, working tree = HEAD + the pins only)

New pins in `ium-app/src/components/CreateJobForm.test.tsx`:

1. **`a guided submit hands the picked region to the vault and hot-applies before the job`** — guides Wasabi, picks `eu-central-2`, submits, asserts `credentialsSet("s3_multipart", { region: "eu-central-2" })` + `applyCredentials("s3_multipart")` both ran **before** `createJob`.

```
× the destination picker > a guided submit hands the picked region to the vault and hot-applies before the job
AssertionError: expected "spy" to be called with arguments: [ 's3_multipart', …(1) ]
Test Files  1 failed (1)
     Tests  1 failed | 6 passed (7)
```

The failure is the bug: `submit()` calls `createJob` and nothing else — the picked region reaches the derived host but never the signing scope (job-time signer reads `env IUM_S3_REGION → vault("region") → "us-east-1"`, `lib.rs`; the draft is never consulted).

2. `a custom submit never writes the vault` — passes trivially at HEAD and stays as the guard: the fix must not write the vault in custom mode.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/105/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"RED posted: {out['html_url']}")
