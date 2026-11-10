# The Loop — full mechanics

*Reference for `zero-to-shipped`. Read when starting/resuming a session or working any issue.*

The loop is a state machine. Every unit of work enters at BOOTSTRAP and exits only at RECORD. Skipping a state is how bugs ship and knowledge dies. This file gives the mechanics and the exact commands for each state, as proven in an 11-day, 267-commit build.

---

## State 1 — BOOTSTRAP (session start)

An agent session (and, honestly, a human's Monday morning) starts with zero memory. The protocol, in strict order:

1. **Locate the working copy and confirm it is the right project.**
   ```bash
   cd <project-root>
   git rev-parse --show-toplevel   # confirm the root
   git status --short | head       # confirm clean-or-known state
   git log --oneline -3            # confirm HEAD matches the record
   ```
2. **Verify push rights before promising anything.** A session that discovers mid-lane that it cannot push is a session that worked for nothing.
3. **Restore secrets** if the environment wiped them (in the source project, the secrets directory was wiped on **seven separate environment recycles** in eleven days — assume yours will be too). Secrets live outside git, `chmod 600`, restored from the owner's vault — never from chat history if a rotated copy exists.
4. **Read the record.** STATUS.md's session log, last 5–10 rows. This is your memory. A chat summary is a *hint*; the record is the *fact*. Where they disagree, the record wins.
5. **Announce the plan** as a numbered TODO derived from (record state + user request), then execute continuously — no pausing to ask permission between steps. If the user said "do X", X is the plan; the record tells you how X is done here.

**Why the order matters:** identity before work (a mis-cloned copy produces great work on the wrong repo); secrets before API calls (half-authenticated debugging wastes an hour); record before plan (the plan may already exist in the backlog).

---

## State 2 — WIRE (establish facts on the wire)

Before designing or claiming anything, read the real thing. Concretely:

- **Source:** open the file, read the section. Do not trust your summary of it from an earlier turn.
- **APIs:** hit the endpoint and print the response. Status codes and bodies, not memory.
- **CI:** pull the run, then the jobs, then the annotations. A red X with the annotation *"The job was not started because recent account payments have failed"* is a completely different problem from a red X with a test failure — and only one of them is yours.
- **Artifacts:** download the artifact and checksum it. "The build said success" is not a fact until bytes match bytes.

The phrase used in the record: **"a wire-read, not a report."** The single most valuable habit in this whole method.

Practical constraints learned the hard way:
- Unauthenticated GitHub API rate limit is 60/h — enough for a *user-view* verification battery, not for bulk metadata work. Keep user-view checks unauth (they prove what a user sees); do metadata checks with the token.
- Short SHAs sometimes fail lookup endpoints; use full SHAs for run/artifact queries.
- Read large outputs through files and slices, not terminal dumps.

---

## State 3 — ISSUE (file the lane)

Open the issue with three parts (template in `templates.md`):

1. **Design note** — the approach, decided *before* code, with the alternatives rejected and why. If you cannot write the design note, you are not ready to write the code.
2. **Acceptance criteria** — the exact evidence that will close it: which gate goes green, which check returns 200, which byte-comparison matches.
3. **Context links** — milestone rung, ADR references, related issues, the owner's directive if any.

Record every lifecycle decision *on the issue*: re-sequencing, parking (with owner sign-off), pivoting (the approach changed mid-lane — say so and why). In the source project, a whole transport milestone was **pivoted** from native engine to a bridge architecture mid-flight; because the pivot was recorded, nobody later wondered why the code looks like that.

Milestone discipline: a milestone is a set of rungs. It closes when every rung closes with evidence. Never "milestone done, issues to clean up later."

---

## State 4 — RED (prove the gap at HEAD)

Produce the failure *before* the fix, on the current tree, and post it to the issue:

- A gate failing (run it, capture output),
- a bug reproduced (minimal steps, actual output),
- a user-view check failing (the 404, the wrong version, the missing key),
- a leak detected (the hygiene gate naming the offending file).

Formats that worked: a short comment with the exact command, the exact failing output (trimmed to the load-bearing lines), and the sentence "RED at HEAD <sha>". This takes minutes and buys three things: you can't fix a phantom, you can't be accused of manufacturing the bug and the cure, and the eventual GREEN is *meaningful* because its opposite was on record.

**Guard-rail learned live:** when you build a new gate, its first RED may be wrong. In the source project a fresh hygiene gate flagged 3 leaks at HEAD — correct — but its clean-fixture self-test also exposed a missing fixture line in the gate itself. Fix the gate first, then post the RED. Evidence produced by a broken instrument is noise.

---

## State 5 — IMPLEMENT (commit-first)

One lane = one coherent commit. Message format (conventional type + the story):

```
fix(release): nothing internal ships public — the provenance sandbox + the user-facing gate
```

Rules that kept the history readable at 267 commits:
- Type vocabulary: `feat`, `fix`, `docs(status)` for record rows, `chore` sparingly.
- The message states the *outcome*, not the activity ("the feed keyed to the hardware that ships", not "changes to updater code").
- If the lane touched a rule or hit a gotcha, the commit (and the STATUS row) records the lesson.
- Never amend-published commits; branch protection rejected a force-push once and *the guard working was the good outcome*.

Commit-first means: the code lands, then the evidence lands on the issue, then the record row lands. Not one giant push at midnight.

---

## State 6 — GATES (the standards are executable)

Run local gates on the **committed tree** (not the dirty worktree — gates on uncommitted state certify nothing):

- format + lint,
- full engine suite, full app suite (in the source project: 166 vitest cases),
- doc lint (the record is code too),
- the specialized gates: updater self-test, publish-hygiene self-test, installer-size budget, a11y suite.

Every gate has `--self-test` and the self-test rides CI. Doctrine and design rules: `verification-and-gates.md`.

**ADR-007 — gates are non-bypassable.** A red gate at 23:50 before a deadline is a stop. Two "we'll skip it just this once" invitations were declined in the source project; both times the gate turned out to be pointing at a real defect.

---

## State 7 — PUSH → VERDICT (chase CI to completion)

Push, then *own* the run:

1. Poll the workflow run until `completed` (the source project used a small poller script printing job verdicts every 45s — a 30-line script, committed to the repo, reused by every session).
2. On GREEN: capture run ID, record it.
3. On RED: job verdicts first, then annotations, then logs of the first failing step. Classify:
   - **code-RED** — the change is wrong; fix forward in a new commit;
   - **infra-RED** — jobs die at start, no logs, identical failure across unrelated commits; see `incident-runbooks.md` (billing/quota is the classic);
   - **flaky-RED** — rerun once to confirm; record the flake either way.
4. Never close a lane while the verdict is unknown. Never tag while RED (ADR-007 again).

---

## State 8 — GREEN → CLOSE

Post the receipt on the issue: run IDs, the checks that passed, the artifacts verified, the byte-comparisons that matched. Close as **completed** (not "wontfix", not silent). If the lane turned out to be unnecessary, close with the *evidence that it was unnecessary* — that's still a receipt.

---

## State 9 — RECORD (the STATUS row)

Append one row to the session log table (template in `templates.md`): date + lane, dense narrative of what happened *including surprises and lessons*, outcome, commits. Commit as `docs(status): ...` and push.

The record compounds: by day 11 the session log held **158 rows** and answered, without hesitation, questions like "why is the updater endpoint a different repo?" and "why do the installers have no attestations?" — the two questions that would otherwise re-litigate themselves every month.

---

## Standing orders (the tempo rules)

- **Continuous execution.** Once the plan is set, execute without pausing for permission at each step. The owner's time is the scarcest resource in solo+AI development.
- **One message = one task.** Don't let a drive-by fix ride along untracked; give it its own issue + row.
- **Verify, then declare.** The words "done" and "shipped" are claims backed by receipts or they are noise.
- **When something bites, it becomes a lesson** in the row — and if the lesson generalizes, into the skill.
