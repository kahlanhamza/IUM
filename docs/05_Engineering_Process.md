# IUM — Engineering Process

> Adopted 2026-11-02 (ADR-007/ADR-008), after an ungated snapshot commit shipped three defects
> (a clippy error, a test that could never pass, fmt drift) and CI never had the chance to speak.
> This document is the workflow contract. It is deliberately short — if a step is skippable, it
> is not listed.

## The loop (every unit of work, no exceptions)

1. **Issue first.** No code without an issue that states the observed behavior, the expected
   behavior (acceptance criteria), and the violated invariant or spec section. Use the issue
   templates (`.github/ISSUE_TEMPLATE/`).
2. **Design note for non-trivial work.** Anything touching the journal schema, the state
   machine, the chunk pool, or the adapter contract gets a short design note in the issue
   (or `docs/` if it deserves an ADR) *before* code.
3. **Failing test first.** For every bug: reproduce it as a test that fails on current HEAD.
   For every feature: the acceptance criteria become tests. Paste the failing output into the
   issue. A fix without a failing test does not ship.
4. **Minimal fix.** Smallest change that makes the failing test pass. No drive-by refactors —
   if you see something unrelated, it gets its own issue.
5. **Full gates — on the committed tree.** Commit first, then `bash scripts/gate.sh`: it refuses
   a dirty working tree, checks out HEAD in a detached worktree, and runs `cargo fmt --all --
   --check`, `cargo clippy -p ium-core --all-targets -- -D warnings`, `cargo test -p ium-core`
   *there*. Working-tree gates are not evidence — defect #11 shipped green local gates on a
   working tree while the committed tree could not compile. CI re-runs everything on
   ubuntu/macos/windows — a red CI row reopens the issue.
6. **One logical commit per issue.** Message: `fix(core): …(#N)` / `feat(adapter): …(#N)` /
   `docs: …`. Reference the issue; never mix unrelated changes. Stage deliberately: after
   committing, `git status` must be clean — anything left behind is an unstaged landmine.
7. **STATUS.md update.** Append a session-log row, move items between Done/In progress/Backlog,
   update §6 tools if the toolbox changed. This is part of DoD, not bookkeeping.
8. **Push.** `bash scripts/gh-push.sh`. A task that is not pushed does not exist — the sandbox
   recycles and context evaporates; GitHub is the memory.

## Definition of Done (checklist for every task)

- [ ] Issue exists with acceptance criteria
- [ ] Failing test written first (for bugs) / criteria-encoded tests (for features)
- [ ] Minimal fix; no unrelated changes smuggled in
- [ ] `scripts/gate.sh` green — fmt + clippy `-D warnings` + full suite, **on the committed tree**
- [ ] CI green on all 3 OS
- [ ] STATUS.md updated (session log + phase pointers)
- [ ] Pushed to `origin/main`

## Session protocol (sandbox recycling)

**Start:** `git pull` → read `STATUS.md` top-to-bottom → `. scripts/rust-env.sh` → `cargo test
-p ium-core` (trust nothing not executed) → pick the top Backlog item.
**End:** STATUS.md row → push → confirm `git status -sb` shows `0` ahead/behind or note why not.

## Rules of engagement

- **Gates are non-bypassable** (ADR-007), and they run on the **committed tree** (defect #11:
  d62094a passed working-tree gates while the pushed tree failed to compile — the gate is now
  `scripts/gate.sh`, which tests HEAD itself in a detached worktree). Snapshot/backup scripts
  (`preserve.sh`) must run the gates or at minimum never be treated as "done" work.
- **Journal is truth** (ADR-002): any state a function acts on is read from the journal, not
  from caller arguments.
- **Honest badges** (ADR-004): nothing reports Done that the adapter did not verify.
- **Real data only** (owner mandate 2026-11-02): reliability claims are proven on REAL files
  (a playable video, not zeros — `tests/real_data.rs`, issue #17) and REAL impaired
  connections (a genuinely throttled socket, not a test clock — issue #18). Synthetic
  fixtures (zeros, wedges, scripted delays) remain useful for offline determinism, but they
  are never the evidence that a transfer claim holds; every milestone must include at least
  one live, real-data proof.
- **Clean-room** (ADR-009): observation of printable strings only; no disassembly, no
  decompilation, no copying of vendor code or assets.
- **Postmortem in 3 lines.** Any bug found outside of tests gets 3 lines in the issue: what
  slipped, why the gates did not catch it earlier, which gate/test now would.
