"""Post the GREEN evidence comment on #112."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## GREEN evidence — the fix, gated on the committed tree

**Commit `487578d`** — `fix(app): the boot gate — the UI polls, it never probes once (#112)`, pushed, `Closes #112`.

The delta (presentation only — zero engine/command/IPC change):

- **NEW `ium-app/src/boot.ts`** — `retryUntil<T>` (first attempt immediate, sleeps only BETWEEN attempts, budget = the shell watchdog's own 8 s, `now`/`sleep` injected so the contract is provable without real clocks) + `waitForEngine` (the verdict carries the engine's own words both ways) + the shared `BOOT_BUDGET_MS`/`BOOT_INTERVAL_MS`.
- **`Launch.tsx`** — poll instead of probe: the ready handoff vs the honest degraded handoff is decided by the budget, not by one race-lost refusal; an unmounted splash retires its verdict (`done = true` in cleanup) so no zombie handoff can fire past the exactly-once contract.
- **`App.tsx`** — the journal reads fire ONLY after the engine answers (the boot.gate contract); a budget spent keeps the honest down pill + toast AND arms a 5 s recovery poll — a cold boot outliving the budget still reaches a working UI without a restart.
- **`CreateJobForm.tsx`** — the adapter fetch rides `retryUntil`; the `registry unavailable` banner now means a registry silent for the whole budget — a REAL registry problem, not a race the engine would have won a second later.

Gates (local, on the committed tree via `scripts/gate.sh`):

```
== gate: committed tree 487578d ==
fmt + clippy -p ium-core -D warnings + cargo test -p ium-core ... GREEN (376)
cargo doc -D warnings ... GREEN
ium-app frontend lane: npm ci + tsc strict + vite build + vitest ... 202 passed (202) — was 191
ium-app rust lane: loud-skip (no WebKitGTK in the sandbox) — owned natively by the CI ium-app job ×3 OS
```

The 11 new tests pin the contract: `boot.test.ts` (retry spacing, budget exhaustion, the verdict mapping, the shared numbers), `launch.test.tsx` (retry → READY handoff), `CreateJobForm.test.tsx` (retry → select fills, no banner; silent budget → banner once, honestly), `boot.gate.test.tsx` (list_jobs fires only AFTER the engine answers; the ready pill; the honest down state with zero journal reads).

Two honest catches during GREEN, both recorded in comments in the code: a frozen virtual clock starves the retry loop of macrotasks (the real clock advances by itself — the virtual one must too), and the budget lets an unmounted splash's verdict resolve post-unmount (zombie handoffs — retired by `done = true`).

Release lane: v0.2.4 is live with this bug on first launch — the fix rides the next cut (v0.2.5) via the updater lane.
"""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/112/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("GREEN comment posted:", c["id"])
