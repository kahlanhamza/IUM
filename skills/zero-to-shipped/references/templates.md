# Templates — the artifacts of the loop

*Reference for `zero-to-shipped`. Copy, fill, ship.*

---

## 1. The issue (file every lane with this shape)

```markdown
## <Milestone rung / lane title> (Closes #<this issue>)

### Design note (decided BEFORE code)
<The approach. Alternatives considered and why rejected.
The ADR(s) this obeys. The constraint that shapes it.>

### Acceptance criteria (the evidence that closes this)
- [ ] <gate X green on the committed tree>
- [ ] <user-view check: endpoint returns Y with Z>
- [ ] <byte-comparison: A matches B exactly>
- [ ] <record: STATUS row appended, lesson captured>

### Context
Milestone: <M# rung N> · ADR: <ADR-###> · Related: #<issues>
```

Rules: no code before the design note reads like a decision. Acceptance criteria are *evidence*, not intentions ("CI green" is weak; "CI run <id> GREEN 8/8 + unauth feed 200 on version X" is evidence).

## 2. RED evidence (comment on the issue)

```markdown
**RED at HEAD `<full sha>`** — the gap is real.

<exact command>
<trimmed failing output — the load-bearing lines only>
<one sentence: what this proves is missing/broken>

Instrument check: gate self-test GREEN (the detector works).
```

## 3. GREEN evidence (comment before closing)

```markdown
**GREEN — closed with receipts.**

- Local gates: <battery> GREEN on committed tree `<sha>`
- CI: run <id> GREEN <n>/<n> (<job list>)
- User-view battery (unauth):
  - <check 1>: <result>
  - <check 2>: <result>
- Bytes: <what matched what, exactly>

Close as completed. Record row: appended.
```

## 4. STATUS.md session-log row

```markdown
| <date> (<lane title>, Closes #N) | <dense narrative: what happened,
surprises, lessons, the owner's directive that drove it, forensics found.
Include the gotchas verbatim — this row is the next session's memory> |
<outcome: verdicts, counts, states> | <commits + tags, both repos if cross-repo> |
```

Rules: **no literal `|` inside cells** (it breaks the table — compose rows in a script if needed). Dense over tidy: a row that reads like a war story is doing its job; a row that reads like a changelog entry is wasted memory. Newest-first ordering.

## 5. Commit message

```
<type>(<scope>): <the outcome, stated as fact> — <the mechanism>, <refs>

Types: feat | fix | docs(status) | chore
```

**Example:**
```
fix(release): nothing internal ships public — the provenance sandbox + the user-facing gate (#102)
```

One lane = one commit. The message says the *result*, not the activity. Record rows ride `docs(status)` commits.

## 6. ADR (ratify every load-bearing decision)

```markdown
## ADR-<NNN> — <decision in one line> ✅ ratified (<date>)

**Context:** <the forces at play, the constraint that forced a choice>
**Decision:** <what we do>
**Alternatives rejected:** <what we don't do, and the one-line why>
**Consequences:** <what this costs us, what it buys us>
**(Amendment <date>):** <what changed, on which issue, and why>
```

## 7. Session bootstrap checklist (top of every session)

```
[ ] working copy at the right remote; HEAD matches the record's last row
[ ] push rights verified (not assumed)
[ ] secrets restored (chmod 600) if the environment wiped them
[ ] tooling present (CLI or curl-fallback confirmed)
[ ] record read: last 5–10 session-log rows
[ ] TODO plan stated (numbered, from record + user request)
[ ] execute continuously — no per-step permission pauses
```

## 8. The unauth battery (release verification script skeleton)

```python
# user-view checks: NO auth (proves what a stranger gets)
# 1. feed resolves unauth
#    GET <distro>/releases/latest/download/latest.json -> 200
#        version == <tag>; platform keys == shipped hardware
# 2. signatures byte-match: manifest entry signature == .sig asset bytes
# 3. checksums: download >=1 real installer per platform;
#    sha256 == published SHA256SUMS.txt lines
# 4. release state: latest / not draft / not prerelease
# 5. source zip: download, list, README current, no internals
# metadata checks (release object, asset list): WITH auth (unauth is 60/h)
```

Every battery result prints; every failure names the check. The script lives in the repo (`scripts/verify_distro_release.py` in the source project) — verification that lives in chat history is verification that didn't happen.

## 9. The release-day checklist

See `release-engineering.md` (the condensed runbook at the end) — copy it into the release issue and tick it in the thread, so the release carries its own receipts.
