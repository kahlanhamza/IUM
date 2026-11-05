# #58 RED evidence — the failing release state at HEAD `f63599c` (2026-11-05)

The release rung's RED is the audit, not a compiler error: the gate "tag is a
promise, not a hope" is not satisfiable at current HEAD. Live checks, run this
session (recycle #7 — toolchain restored, suite re-proven 258/258 first):

## 1. The promised tag does not exist

```
$ git tag -l
(empty)
```

## 2. No GitHub release exists

```
GET /repos/kahlanhamza/IUM/releases → []
```

## 3. The release section #58 defines does not exist in CHANGELOG.md

The file holds TWO sections that cannot both be 0.1.0:

- `## [Unreleased]` — carries M5→M10 content (#17–#57, #63) including the
  entire M6/M7/M8 work — a release section pretending to be unreleased.
- `## [0.1.0] — 2026-11-02` — the M5-era frozen-facade SEED (#20), holding
  only M1–M4 content. Two sections for one version violates Keep-a-Changelog;
  neither is the release section the issue defines.

## 4. Changelog audit: 16 closed issues have NO entry at all

Bullet census over the whole file (`^- #(\d+)`): 38 bullets, 37 distinct
issues. Closed issues missing from the record:

`#21, #31, #34, #35, #37, #38, #41, #42, #43, #44, #45, #46, #51, #52, #53,
#54`

— the M6 journal-seam and e2e rungs (#34/#35), ALL of M7 (#41–#46), M8 rungs
4–6 (#51/#52/#53), and the CI-debt/flake/process work (#21/#31/#37/#38/#54).
This violates the repo's own #20 rule (changelog-in-same-commit discipline;
#57's rung repaired #63's miss but did not audit the backlog). The cut writes
these entries now, sourced from STATUS.md rows only.

## 5. Versions (the ONE already-green part)

- workspace `Cargo.toml` `version = "0.1.0"`; `ium-core` inherits
  (`version.workspace = true`).
- `ium-app/src-tauri/Cargo.toml` `version = "0.1.0"`;
  `tauri.conf.json` `"version": "0.1.0"`.
- `ium-e2e/Cargo.toml` `version = "0.1.0"`.

## 6. Gate preconditions (verified before the RED is actionable)

- #56 closed; #57 closed (board repair this session: the landing commit
  `f63599c` said `refs #57`, not `Closes #57` — GitHub never auto-closed;
  evidence comment `6663793835` + explicit close).
- `cargo test -p ium-core` → **258 passed; 0 failed** on the committed tree.
