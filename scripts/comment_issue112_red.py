"""Post the RED evidence comment on #112 (the boot race)."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## RED evidence — executed at HEAD `ed29249` (2026-11-14), full vitest output

```
FAIL src/boot.test.ts — the helper does not exist (module missing = the contract is unimplemented)
FAIL src/boot.gate.test.tsx > list_jobs fires only AFTER the engine has answered
    AssertionError: expected 1 to be greater than or equal to 3
    → list_jobs fired on the FIRST (race-lost) probe; the engine never answered
FAIL src/boot.gate.test.tsx > a boot that resolves paints the ready pill
    AssertionError: expected 'New Upload…' to contain 'Connected'
    → one failed probe pins the pill down forever at HEAD
FAIL src/boot.gate.test.tsx > a budget spent without an answer renders the honest down state
    AssertionError: expected 1 to be 0
    → the journal was read DURING the boot race (list_jobs on a dead engine)
FAIL src/launch.test.tsx > retries a boot-race refusal and hands off through the ready path
    AssertionError: expected '…The engine did not answer — opening IUM anyway' not to contain 'opening IUM anyway'
    → the splash completed DEGRADED even though the engine answered on attempt 3
FAIL src/components/CreateJobForm.test.tsx > retries a boot-race refusal — the select fills, no banner, no onError
    → the banner rendered verbatim: "registry unavailable: Error: state not managed for field
      'engine' on command 'list_adapters'" and the select never filled (options 0 ≠ 3)
FAIL src/components/CreateJobForm.test.tsx > a registry that never answers within the budget still speaks — once, honestly
    AssertionError: expected 1 to be greater than 1
    → one-shot: no retries before the banner
Test Files 4 failed (4) | Tests 6 failed | 11 passed
```

The failing assertions ARE the owner's v0.2.4 screenshot, executed as tests: list_jobs during the boot gap, the degraded splash, the `registry unavailable` banner.

Next: implement `src/boot.ts` (retryUntil + waitForEngine, timing-injected) and move all four boot probes onto it.
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/112/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("RED comment posted:", c["id"])
