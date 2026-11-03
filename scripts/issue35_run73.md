## Run #73 evidence + the leg-B row fix (content-addressed rows)

**The inspector now opens** (run #72's fix worked) and leg A is fully green
including the drawer's chunk-table asserts. Run #73 died one step later, in leg
B's running-wait:

```
ium-e2e/tests/e2e_shell.rs:697:10:
job B running: "row never reached `running` (pct cap None); last state=done pct=100"
```

with recorder B showing `{"put_count": 1, "received_bytes": 16777216, …}` — the
PUT **did** run at its 1 MiB/s throttle (16 MiB ≈ 16 s, inside the 30 s window)
and completed honestly. The test was watching **the wrong row**.

**Mechanism:** `JobManager::list_jobs` renders NEWEST-FIRST (the manager's
documented order), and `App.tsx::refreshJobs` replaces the array wholesale — so
the instant job B was created, the table re-ordered to `[B, A]`. Leg B's
`row_b = "table tbody tr:last-child"` selector — correct for the one-row world
it was written in, never executed until now — pointed at **job A's row**
(`done pct=100`), while job B ran in row 1. Positional selectors flip the
moment a second row exists.

**Fix (harness-only):** rows are now addressed BY CONTENT — `row_for_file()`
matches the row whose first cell holds the source's basename (`JobsTable`
renders it) and returns that row's `nth-child` selector. Leg A and leg B both
use it; no positional assumptions remain anywhere in the suite.

Failure chain so far, each face killed in turn: #70 abort at the Upload click
(#39) → #71 row frozen at `probing pct=0` (#40) → #72 inspector row click
(geckodriver#1294-class, cell click) → #73 leg B watched job A's row (this fix).
Remaining unexercised steps after this: cancel mid-PUT → Canceled badge →
recorder holds nothing → HEAD 404.

Side note on run #73: `core (macos-latest)` shows `cancelled` with its log blob
missing (BlobNotFound) — the runner died mid-job; infra noise, not code (the
same job was green on #71/#72 with identical sources; `cancel-in-progress` is
already `false` since 940ddc9).
