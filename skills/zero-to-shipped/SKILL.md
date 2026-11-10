---
name: zero-to-shipped
description: |
  The complete operating system for taking a software project from an empty
  repository (day 0) to a shipped, auto-updating, multi-platform product — in
  days, not months — using an issue-first, evidence-first engineering loop
  that works like a big-tech team of one human + one AI agent. Use this skill
  for ANY greenfield project, rewrite, rescue, or "let's build X" conversation;
  whenever a session starts on an in-progress project that follows this loop
  (STATUS.md, issues, gates, RED/GREEN evidence); whenever you are about to
  cut a release, publish publicly, handle secrets/tokens, debug CI that fails
  instantly, recover from an environment recycle, or decide what to build
  next. If the user says "ship it", "cut a release", "make it public", "it's
  broken in CI", "start a new project", "what's next", or asks how the team
  works — this skill triggers. It encodes the full method proven over an
  11-day build: 267 commits, 100+ closed issues, 15 milestone tiers, 7 tags,
  12+ ratified ADRs, and a public product surface with zero source leakage.
---

# zero-to-shipped

**The method: how one human and one AI agent shipped a real desktop product in 11 days, and how you can run any project the same way.**

This is not a style guide. It is an operating system. It was extracted from a real project — IUM, a reliability-engineered desktop upload engine (Rust engine daemon + Tauri 2 shell) — that went from an empty repository on day 0 to a shipping, auto-updating, three-platform product with a public distribution surface on day 11: **267 commits, 100+ issues driven to closure, 15 milestone tiers, 7 tags (v0.1.0 → v0.2.2), 12+ ratified architecture decisions, a 158-row session log, and 14 engineering documents** — with zero secrets leaked and zero source exposed on the public surface.

Everything in here was earned by hitting the problem for real. When you see a rule, there is a war story behind it (see `references/war-stories.md`). When you see a checklist, it exists because skipping its step once cost an hour or broke users.

Read this file top to bottom once. After that, the reference files are your deep dives:

| File | Read it when |
|---|---|
| `references/the-loop.md` | Starting or resuming any session; filing or closing any issue |
| `references/release-engineering.md` | Cutting ANY release (tag choreography, provenance hygiene, the unauth verification battery) |
| `references/verification-and-gates.md` | Building/changing any gate, test, or verification claim |
| `references/incident-runbooks.md` | CI fails instantly, quota/billing blocks, tokens, sandbox recycles, broken releases |
| `references/product-surface.md` | Making a public-facing repo/README/distro; preventing source leakage |
| `references/war-stories.md` | Before dismissing a rule as bureaucracy; when teaching this method |
| `references/templates.md` | Writing the issue, the evidence comment, the STATUS row, the commit, the ADR |

---

## 0. The thesis (what "like big tech" actually means)

Big tech teams are not slow because of process. They are reliable because every unit of work is **traceable, evidenced, and recorded** — so anyone (a new hire, a reviewer, an auditor, or a future you with no memory) can reconstruct *why* the code looks the way it does. The failure mode of solo + AI development is the opposite: brilliant bursts of code with no trail, no evidence, no record — until something breaks and nobody knows what changed.

This loop gives you big-tech reliability at one-human speed. The pillars:

1. **Issue-first.** Nothing is built that isn't tracked. Every unit of work starts as an issue with a design note and acceptance criteria.
2. **Evidence, not claims.** "It works" is worth nothing. RED proof at HEAD, then GREEN proof after the fix, posted where the team can see it.
3. **Gates, not vibes.** Every standard becomes an executable gate that fails loudly. A standard without a gate is a wish.
4. **The record.** One living document (STATUS.md) is the single source of truth, appended every task. Amnesia — human or machine — is solved by the record, not by memory.
5. **Wire facts.** Never trust a summary, a report, a memory, or an assumption. Read the actual source, the actual log, the actual API response. A "wire-read" beats a "report" every single time.
6. **User-view verification.** Verify from exactly where the user stands — unauthenticated, plain downloads, real installers — not from inside your privileges.

---

## 1. The numbers (what this method delivered in 11 days)

Memorize the *shape*, not the digits — your project will differ. The shape is: **spec-first day 0, working engine by day 4, first release by day 5, product surface by day 6, polish + hardening to day 8, release engineering bar to day 10, public distribution with zero leakage by day 11.**

- **Day 0 (2026-11-01):** Architecture teardown of five competing products (aria2, XDM, Motrix, FDM, IDM) as clean-room evidence, then the MVP spec. Zero code. The first two commits are documents.
- **Day 4:** Engine milestones M0–M5 accepted: journal, chunk pool, verify-before-declare, slow-connection eviction.
- **Day 5 (v0.1.0):** First cut. S3 multipart, connection bonding, FEC, zstd — all gated, all evidenced.
- **Day 6:** Product UI pass, per-OS installer pipeline, credentials vault, design system + a11y gate, error-copy system, the logomark, a 100-jobs load e2e, an installer-size budget gate, the brand sheet.
- **Days 7–8:** Transport lanes (YouTube, Google Drive, MEGA, TeraBox...), the visual identity system (two inks, one type system), ⌘K command palette, curl hardening sweep, then **v0.2.0 + branch protection**.
- **Days 9–10:** The release bar: upgrade-path guarantee (v(N-1) resumes byte-exact), nightly dependency audit, OS life-event handling, docs arc, then **v0.2.1 with a 13/13 CI matrix and an independent sha256 cross-check**.
- **Day 10–11:** The distribution move: updater feed keyed to shipped hardware (#100), private source + public distro split (#101) — including billing forensics that proved three "failing" CI runs were never code — and the provenance hygiene lane (#102) that stripped 18 internal sidecars from a 33-asset public release down to 15 clean assets.
- **Day 11:** The public product surface finished like a big-tech marketing page — adaptive logo, screenshots, user guide — and verified from the unauthenticated user's chair.

The point of showing the days: **the sequence matters more than the speed.** Spec → engine → release → product → hardening → distribution. Skipping ahead is how projects rot.

---

## 2. The loop (every unit of work, no exceptions)

This is the heart. Internalize the order: **bootstrap → wire → issue → RED → implement → gates → push → verdict → GREEN → close → record.** Full mechanics with commands live in `references/the-loop.md`.

### 2.1 Bootstrap (before ANY work in a session)

You (the agent) wake up with no memory. Do not pretend otherwise. In order:

1. **Restore identity:** confirm the working copy, the remote, and that you can push. Verify — don't assume.
2. **Restore secrets** if the environment wiped them (they will be, repeatedly — see `references/incident-runbooks.md` → "sandbox recycle").
3. **Read the record:** the last STATUS.md session-log rows. That IS your memory. Never start work from a chat summary alone.
4. **State the plan** as a numbered TODO derived from the record + the user's request, then execute without pausing to ask permission at every step (the standing order: continuous execution).

### 2.2 Wire facts (the anti-amnesia reflex)

Before designing, implementing, or claiming anything, read the actual thing:

- The actual source file, not what you remember about it.
- The actual API response, not what the docs say it should be.
- The actual CI log / check-run annotation, not the red X.
- The actual bytes on disk (checksums, sizes, dumps), not what the build claimed.

**Why:** in 11 days this reflex caught: three CI "failures" that were actually a billing block (the jobs never started), an updater feed keyed to hardware the product doesn't ship (#100), a release carrying 18 internal sidecars nobody audited (#102), a release tag that had silently drafted itself, and a source zip that was stale relative to main. Every one of those would have shipped wrong on a "trust the report" model.

### 2.3 Issue-first (nothing untracked)

Every unit of work — feature, bug, spike, hygiene lane — is an issue with: a **design note** (the approach decided *before* code), **acceptance criteria** (what evidence will close it), and links to the milestone/ADR it serves. Re-sequencing, parking, and pivoting are recorded on the issue, not decided silently. Milestone vocabulary ("rungs") keeps the roadmap honest: a milestone closes when its rungs close, not when it feels done.

### 2.4 RED evidence (prove the gap before filling it)

At HEAD, on the real tree, produce the demonstration that the standard is currently unmet — a failing gate run, a reproduced bug, a 404 from the user's chair — and post it to the issue. This is the discipline that separates "I think we're missing X" from "X is missing, here is the receipt." It also protects you from fixing things that aren't broken.

### 2.5 Implement commit-first

Write the fix, land it as one coherent commit with a message that tells the story (`fix(release): nothing internal ships public — the provenance sandbox + the user-facing gate`). One lane = one commit (plus record commits). If you need to describe two changes in one message, you had two lanes.

### 2.6 Gates (the standards are executable)

Local gates run on the **committed tree** before push: format, lint, full engine test suite, app test suite (166 vitest cases), doc lint, and the specialized gates (updater self-test, publish hygiene self-test, installer-size budget, a11y). Every gate has a `--self-test` that proves the gate itself works, and that self-test rides CI so gates can't rot silently. **Gates are non-bypassable (ADR-007):** a red gate is a stop, never a "skip it this time."

The deep doctrine — detectors vs. stories, fixture-on-real-shapes, and the two times the escaped-regex trap was caught *by the gate* — is in `references/verification-and-gates.md`.

### 2.7 Push → chase the CI verdict to completion

Pushing is not the end. Poll the workflow run until it completes; if RED, read the *job* verdicts and their annotations, and distinguish **code-RED** (your change) from **infra-RED** (jobs dying at start — billing, quota, runners). Never leave a run un-triaged overnight; never mark a lane closed on a yellow or unknown verdict. The runbook for instant-all-fail signatures is in `references/incident-runbooks.md`.

### 2.8 GREEN evidence → close

Post the receipt on the issue: what was verified, the run IDs, the exact checks. Then close as completed. An issue closed without evidence is a rumor.

### 2.9 Record (STATUS.md row, every task)

Append one row to the session log: date + lane, what actually happened (dense, factual, including the surprises), outcome, commits. Then a record commit + push. **ADR-008: STATUS.md is the single source of truth, pushed every task.** The record is what makes agent amnesia (and human amnesia, and context-window compaction) survivable. If a rule gets violated or a gotcha bites, the lesson goes in the row — the record is where war stories are born.

---

## 3. The architecture doctrine (what we built and the shape to copy)

The product: a desktop upload engine where the **engine is the product** and the UI is a window onto it. Decisions were ratified as ADRs *before* implementation, each with its reasoning preserved. The transferable shape:

1. **Engine first, UI frozen (ADR-006).** Build the reliability core before the chrome. The UI frozen during engine milestones prevented endless visual churn from eating the hard weeks. (Amended once, by recorded decision, when the UI became the product surface — amendments are fine; silent drift is not.)
2. **The journal is the single source of truth (ADR-002).** SQLite (WAL) records every byte; crash-safety and resume derive from replaying the journal, not from memory. State that lives anywhere else is decoration.
3. **Verify-before-declare (ADR-004, ADR-010).** The engine never claims an upload succeeded; it verifies (E-tag/hash compare) and only then declares. The same doctrine applies to *your pipeline*: never declare a release shipped until you've downloaded it back and byte-checked it.
4. **Adapters declare, planner obeys (ADR-011, ADR-012).** Each transport adapter declares its capabilities (resumability, parallelism, chunk size); the planner never assumes. Capability discovery over hard-coded assumptions — this kept ten transports (S3, WebDAV, Google Drive, YouTube, Terabox, Feishu, Mega, HTTP, TUS, rclone-bridge) from forking the core.
5. **Clean-room policy (ADR-009).** Study competitors, write the evidence doc, implement from the spec — never from their code. The REA research brief + resistance checklist make this auditable.
6. **Bridges with seams.** When a transport is better borrowed than rebuilt (rclone, MEGA, TeraBox), build a *seam* — a narrow, declared interface — not a fork of the core. Pivot decisions (engine → bridge) are recorded as pivots with reasons.
7. **The updater chain as a trust system.** Signed manifests (minisign keyring pinned in the app), per-platform entries, `.sig` sidecars for every installer, platform keys that match the hardware you actually ship (darwin-aarch64, linux-x86_64, windows-x86_64), and a feed hosted independently of the source repo's visibility.

---

## 4. Release engineering (the bar)

The full playbook with checklists is `references/release-engineering.md`. The bar in brief:

1. **Version alignment sweep** — every manifest (workspace, member crates, package.json, tauri.conf, Cargo.lock) moves together. Twice in this project a stray manifest lagged behind (a shell crate left at 0.1.1); the sweep exists because of that.
2. **Honest CHANGELOG.** Semver by content, not by mood: a security fix + docs arc is a *patch*, and the record says so. No fake features.
3. **CI GREEN before tag (ADR-007).** No tag rides a red or unknown matrix. No exceptions, including "docs-only" commits — if you can't run CI, you can't tag.
4. **Tag choreography.** Tag → release run → publish → **unauth verification battery** → issue closure. Never delete or move the tag of a *published* release: deleting a tag flips its release to draft (the update feed goes dark) and recreating the ref does NOT re-publish. Move tags only for pre-release fixes, then re-publish immediately and re-verify.
5. **Provenance hygiene.** SBOMs and attestations are *internal* artifacts — they embed private repository identity, workflow paths, commit SHAs. They ride private CI artifacts, never the public release. The publish step hard-fails if any provenance-shaped file appears in the public asset set. This one rule is the difference between a public release and a source leak.
6. **The unauth battery.** After publish, verify as a user with no credentials: the feed URL resolves (200, correct version, platform keys = shipped hardware), signatures byte-match the published `.sig` assets, real installer checksums match the published SHA256SUMS, and the release is the repo's "latest". This battery caught the platform-key bug (#100) that would have broken every v0.2.0 install's updater.

---

## 5. The public surface (private source + public distro)

Full guide: `references/product-surface.md`. The pattern: the source repo stays private; a separate **public distro repo** is the product's face — description, topics, product README, user guide, sanitized changelog, releases with the clean asset set. Rules that were learned in public:

- The README explains **what it does and how to use it** — never how it's made. No issue numbers, no internal jargon, no architecture internals, no team vocabulary.
- **GitHub's auto-generated "Source code (zip)" archives the release repo** — so the distro repo's own README is what leaks or doesn't. Keep it product-only and keep it current relative to the tag (a stale tag ships a stale README into every zip).
- Dark mode is the default for developers: adaptive logos via `<picture>` + `prefers-color-scheme` (dark ink for light surfaces, white ink twin for dark surfaces). A dark logo on GitHub's dark canvas is invisible — and it was the *first* thing the owner noticed.
- Screenshots are taken through a real harness against the real app — not mockups.
- The one-way rule: every asset that goes public passes an allowlist. Everything else stays private.

---

## 6. Incident doctrine (you WILL hit these)

The runbooks with exact commands: `references/incident-runbooks.md`. The five you must know cold:

1. **CI all-red in seconds = infra, not code.** Read check-run annotations ("job was not started because recent account payments have failed..."). Distinguish instantly-failing jobs from actually-running ones. On private repos, Actions quota/spending-limit blocks are the classic cause; the recorded fix was a temporary public window (public repos' standard runners are free) plus the Dec 1 quota reset.
2. **Sandbox/workspace recycle.** Secrets vanish; long-running processes die; the git state survives. The restore protocol (identity → push rights → secrets → record) is muscle memory. Store nothing critical outside the repository or the record.
3. **Secrets discipline.** Local-only, `chmod 600`, gitignored AND excluded via `.git/info/exclude` (belt and braces), never printed, never committed — and when a token has appeared in chat history even once, rotation is advised, *and every dependent secret must rotate in step* (a publish token and the repo secret holding the same value must change together or the next publish fails).
4. **The feed went dark.** Release 404ing unauth? Check draft status first. Tag deletion drafts the release. PATCH `draft: false` restores; then re-run the unauth battery.
5. **The trap cabinet.** Escaped-regex pins that `replace_all` misses (bitten twice, caught by gates twice); short SHAs failing lookups; heredoc trailing commas turning strings into tuples; filename drift (`SHA256SUMS` vs `SHA256SUMS.txt`); unauth rate limits (60/h) breaking verification scripts mid-run. Every trap has a detector or a recorded lesson — add yours to the cabinet.

---

## 7. How to adapt this to a new project (day 0 checklist)

1. Create the repo. Commit **documents first**: architecture teardown of 3–5 competing tools (clean-room evidence), then the MVP spec with acceptance criteria per milestone.
2. Ratify the ADRs that matter for your domain (language, storage, the verify-before-declare equivalent, engine-first ordering, gates non-bypassable, the record).
3. Stand up the record: STATUS.md with the session-log table. First row = day 0.
4. Stand up CI from commit one (format + tests + doc lint). Add gates as standards appear; every gate gets a self-test.
5. File milestone M0–M5 as issues with acceptance criteria. Start the loop (§2). Don't touch the UI until the engine milestone set is accepted (or record the amendment if your project is UI-first).
6. Cut v0.1.0 when the engine bar is met — the loop is the same, just smaller.
7. From the first release onward, run the full release bar (§4). From the first public surface onward, run the leak rules (§5).
8. Append to the record every task. When something bites, the lesson goes in the record — and, if you maintain a skill like this one, into the war stories.

---

## 8. What "not done yet" means

The project ships, and the backlog lives in the record with the same discipline: every future lane gets an issue, RED evidence, a commit, gates, a verdict, GREEN evidence, closure, and a row. The method doesn't end at v1 — it *is* the project.

**Final calibration:** the loop feels heavier than hacking. It is heavier — by about ten minutes per task. In 11 days it bought: zero lost work across seven environment recycles, zero leaked secrets, zero false "shipped" claims, three releases where every asset was byte-verified, and a record that lets a stranger (or a fresh agent instance) reconstruct every decision. That trade is the whole skill.
