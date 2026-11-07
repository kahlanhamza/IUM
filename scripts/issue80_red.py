#!/usr/bin/env python3
"""Post RED evidence on #80."""
import json
import urllib.request

TOKEN = open("/home/z/my-project/.secrets/gh_token").read().strip()
HDRS = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"}

body = """## RED — the updater gate fails at HEAD (evidence, #80)

New gate: `ium-app/src/updater.test.tsx` (6 contracts — honest degrade, the About
state machine, the consent rule, error-as-a-sentence).

At HEAD `src/updater.ts` does not exist and the About block has no update control:

```
Error: Failed to resolve import "./updater" from "src/updater.test.tsx". Does the file exist?
 Test Files  1 failed (1)
      Tests  no tests
```

The missing-module RED (the #69/#70 pattern). The swap follows in the same commit
(Closes #80)."""

data = json.dumps({"body": body}).encode()
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/80/comments",
    data=data, headers=HDRS, method="POST")
print("RED evidence:", json.load(urllib.request.urlopen(req))["html_url"])
