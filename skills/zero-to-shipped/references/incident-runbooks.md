# Incident Runbooks

*Reference for `zero-to-shipped`. Read when something is on fire.*

Each runbook: **signature → diagnosis → treatment → prevention.** All were executed for real in the source project (several more than once).

---

## RB-1: CI all-red in seconds (billing / quota block)

**Signature:** every job fails within seconds of push; no logs; identical failure on unrelated commits (even `fmt`). The run shows `completed/failure` with 8/8 jobs dead but *nothing* actually ran.

**Diagnosis:** read the check-run annotations, not the red X:
> *"The job was not started because recent account payments have failed or your spending limit needs to be increased."*

Jobs that never started have no useful logs — the annotation is the only truth. Also read job `started_at`/`completed_at`: a job that lived for two seconds never compiled anything.

**Treatment options (in order of preference):**
1. **The public-window play:** on GitHub, standard-runner minutes on *public* repos are free and unmetered — a temporary visibility flip clears the block instantly and burns no quota. (Source project: flipped public → GREEN 8/8 on the same commit that was infra-dead minutes earlier.) Plan any pending releases/CI-heavy lanes into the window; verify everything *inside* the window; flip back when done.
2. Raise the spending limit (owner's billing decision, not the agent's).
3. Wait for the monthly quota reset (and schedule next month's private-period usage *under* the quota — the source project burned 5,708 units against a 2,000/month tier in five days of private-period CI).

**Prevention:** know your quota math (`scripts/quota_window.py` in the source project measured the burn). Schedule heavy lanes while public/free. Treat "3 unrelated commits all red at start" as an infra signature before suspecting code — **the code was fine three times before anyone looked at annotations.**

**Post-block rule:** the first CI run after the block clears is the real verdict for whatever landed during the outage. If a lane was closed on "pushed but CI infra-blocked", its verdict is *pending*, not green — and no tag rides a pending verdict (ADR-007).

---

## RB-2: Sandbox / workspace recycle (agent amnesia)

**Signature:** the session's environment is fresh: secrets directory empty, background processes gone, `gh` CLI (or similar tooling) not installed, but the project directory intact.

**Diagnosis:** nothing is wrong — this is the environment's lifecycle. In the source project this happened **seven times in eleven days**. The damage is bounded by one rule: *nothing critical lives outside the repo or the record.*

**Treatment (restore protocol, in order):**
1. Identity: working copy at the right remote; `git log` matches the record's last row.
2. Push rights: push an empty commit or check collaborator status *before* starting a lane.
3. Secrets: restore from the owner's vault (they are stored outside the sandbox precisely because of this runbook). `chmod 600` immediately.
4. Tooling: reinstall what the lane needs (`gh` or fallback to raw `curl` + API; the fallback was used for an entire session — prefer whichever is present, don't stall).
5. Read the record's last rows before planning. **The record is the memory** — a recycled session that reads STATUS.md is fully operational in minutes; one that trusts a stale chat summary re-does finished work.

**Prevention:** store all state in the repository (scripts, record, docs) or in the owner-held vault (secrets). Never in `/tmp`, never in the process table, never "in the chat."

---

## RB-3: Token hygiene (leak suspicion, rotation, coupled secrets)

**Signature:** a credential appeared in chat history, a log, a screenshot — even once. Or: rotation is due and one token feeds more than one secret.

**Diagnosis:** assess exposure honestly. In the source project, the same token value was re-pasted by the owner across restores, so it entered chat history repeatedly → **rotation advised** (owner's action, at a calm moment). This is normal hygiene, not a fire alarm — *if* the repo history is clean.

**Treatment:**
1. Verify the history is clean (the source project pickaxe-audited every commit for token patterns before any public flip: zero real tokens; the only `github_pat_` match was the pre-commit *block* regex — the guard working).
2. Rotate at the provider; update **every** dependent secret in the same window. The coupled-secret trap is real: the publish job used a repo secret whose value equaled the personal token — rotating one without the other breaks the *next* release, invisibly, weeks later.
3. Re-verify a write operation after any rotation (a cheap API call with the new secret).

**Standing discipline (belt and braces):**
- secrets live in a local-only directory: `chmod 600`, in `.gitignore` **and** `.git/info/exclude` (the second copy survives `.gitignore` edits),
- never printed, never committed, never echoed into logs,
- a pre-commit hook blocks token-shaped strings (it earns its keep the first time a paste goes wrong),
- rotation cadence: after any exposure, after any owner-requested paranoia, and "at the next calm moment" otherwise.

---

## RB-4: The release/feed went dark (draft flip, stale archive)

**Signature:** the updater feed or release page 404s *unauth* while the release object still exists via API.

**Diagnosis (ordered, cheapest first):**
1. `GET /releases` (authed) → is `draft: true`? → **tag deletion drafts the release.** Deleting/moving a published release's tag ref is the trigger; recreating the ref does *not* re-publish.
2. If not draft: is it prerelease? Is it actually the repo's `latest` (the `latest` resolution excludes drafts and prereleases)?
3. Is the asset itself gone vs. the archive CDN lagging? (Source zips regenerate on tag move with ~a minute of CDN lag — verify with an attempt-loop, not one curl.)

**Treatment:** restore draft → published:
```
PATCH /repos/<owner>/<repo>/releases/<id>  {"draft": false, "tag_name": "<tag>"}
```
then **re-run the full unauth battery** (release engineering, phase D). The source project recovered a 404-dark feed in ~2 minutes with this exact runbook.

**Prevention:** treat published release tags as **immutable infrastructure**. Need a different tree in the source zip? Ship it in the *next* release, or (if truly needed) re-point the tag, PATCH out of draft immediately, re-verify — and record the outage window in the session log.

---

## RB-5: "It's broken" reports that aren't (the false alarm)

**Signature:** an angry "this leaked / this is broken" report about something that *looks* wrong but isn't (source project: the auto "Source code (zip)" on a release page, which the owner read as a source leak — it actually archives only the release repo's README).

**Treatment:**
1. **Take it seriously anyway** — the underlying suspicion is often right even when the report is wrong. In the false-alarm case, the audit it triggered found 18 genuinely-internal sidecars in the public release. The report was wrong; the instinct was correct.
2. Produce the wire-fact (download the zip; list its contents; show the bytes).
3. Fix the *real* gap the report exposed (there was one: the zip's README needed to exist and be current).
4. Never argue the false alarm before doing 1–3.

---

## RB-6: The trap cabinet (small, sharp, recurring)

Each of these bit in the source project. Each is now a detector or a reflex:

| Trap | Bite | Reflex |
|---|---|---|
| **Escaped-regex replace-all** — `/0\.2\.0 is available/` survives a version sweep because of the literal backslash | twice | grep for the *escaped* form (`\d\.\d`) in test pins after any version bump; the test gate catches it pre-push — both times it did |
| **Stray trailing comma in a heredoc** — `body = """..."""` + `,` makes a tuple; downstream `TypeError` | once | keep the payload construction in a saved script (per script-persistence rules), not a one-liner |
| **Short SHA lookups** — runs/artifacts queried by 7-char sha return nothing | once | `git rev-parse HEAD` for full sha on every API query |
| **Filename drift** — `SHA256SUMS` vs `SHA256SUMS.txt` 404s mid-battery | once | the verification script asserts the exact asset names from the release object, not from memory |
| **Unauth rate limit** — 60/h breaks a long verification battery at step N | once | user-view checks unauth, metadata checks authed; the battery resumes, doesn't restart |
| **CLI missing after recycle** — `gh: command not found` mid-incident | twice | fallback to `curl` + REST is always available; write the incident script against the API, not the CLI |
| **Branch protection force-push rejection** | once (by design) | never plan to rewrite pushed history; the rejection is the guard *working* |
| **Stale local HEAD** — tagging/pushing from a tree behind the remote | risk | `git pull --ff-only` before every tag and every push |
| **Table cell containing a literal `\|`** — breaks the STATUS row table | near-miss | compose record rows in scripts that escape pipes |

Add your own bites to this cabinet with the same columns. The cabinet is the cheapest insurance in the whole method: every entry cost one incident once and should never cost two.

---

## The meta-rule

**Every incident ends with three artifacts:** the fix (commit), the verdict (GREEN evidence), and the lesson (record row → runbook/cabinet/skill). An incident without a lesson update *will* repeat — the seven recycles, the two escaped-regex bites, and the twice-taught manifest-straggler lesson each stopped repeating only when they were written down.
