# Issue #75 (Lane 7) — design note: the telemetry stance

Lane 7 of #66, owner decision recorded this session: **"I approve lane 7, do what
you need to do, we always follow big tech teams behaviors and architecture so we
must ship a real big tech products that users trust."** That is #66 option (a)'s
consent model — anonymous diagnostics, **opt-out** (the big-tech standard: visible
switch, first-class off, honest disclosure).

## Wire facts (read from the tree, none assumed)

- **W1 — the lane contract** (#66 §7): "telemetry means a privacy stance (what is
  collected, opt-in vs opt-out, where it lands)... Each is a defensible big-team
  answer; silence is not." Acceptance bar: "The telemetry stance is WRITTEN
  (whatever it is) and shipped accordingly." The lane's deliverable is therefore
  a WRITTEN stance the shipped product provably matches.
- **W2 — the journal carries user data in plain text** (`journal.rs::JobRecord`):
  `source_path`, `endpoint`. Any telemetry derived by copying journal rows would
  leak file names and endpoints. The event model must be built from a field
  ALLOWLIST, never from records.
- **W3 — the observation points already exist**: `spawn_watcher`
  (lib.rs:283–297) holds the FINAL snapshot at the terminal transition
  (adapter / state / source_size / created_at / updated_at — path and endpoint
  sit on the same record and must NOT ride along); `setup` (lib.rs:299) is the
  single boot point where a panic hook can be installed before risky work.
- **W4 — the house storage pattern** (`vault.rs`, #69): config-dir file, 0600,
  atomic tmp+rename, corrupted-file-is-default honesty, and a STANDALONE
  `rustc --test` harness because the sandbox has no WebKitGTK. vault.rs needs
  extern flags for serde — a pure-std module (hand-rolled, correctly escaped
  JSON for 5 fixed fields) needs none; the harness becomes trivially honest.
- **W5 — the drift-gate pattern exists** (`brand.test.ts` D2, #73): a written
  document parsed and required EQUAL to code constants. The charter gets the
  same gate — the stance cannot drift from the implementation.
- **W6 — the voice gate pattern exists** (brand.test.ts D3): new user-visible
  strings must join the enumerated copy surface (capitalized, no exclamation
  marks, no emoji, IUM spelled IUM).
- **W7 — the transport reality**: no HTTP client in the shell (reqwest lives in
  ium-core; adding one grows the installers the budget gate #72 now pins); NO
  sink endpoint exists (no Sentry DSN, no self-hosted collector — that is an
  owner credential the repo cannot invent). The lane-4 precedent governs:
  signing = pinned owner ask, installers shipped unsigned+checksummed. Here:
  the sink = pinned owner ask, the pipeline ships complete UP TO the network
  boundary — and because no transport code ships, "where it lands" is answerable
  with a file path, the strongest trust answer available this build.

## Design

- **D1 — the stance is WRITTEN; the charter is the pivot.**
  `docs/11_Telemetry_Stance.md` records the decision (option (a), opt-out), the
  collected-fields table, the never-collected list, where events land, the
  switches, the honest limits, the gates, and how the stance may change. The
  #66 acceptance bar is satisfied by this document + the enforcement below.
- **D2 — the allowlist is the trust gate; the charter cannot drift.**
  Every event carries ONLY charter-table fields. `job_terminal`: `v`, `kind`,
  `ts`, `adapter` (registry key), `state` (terminal state name), `size_bucket`
  (5 buckets), `duration_bucket` (6 buckets). `panic`: `v`, `kind`, `ts`,
  `message` (verbatim payload — the documented trade, see D6), `source`
  (panic Location file — an IUM source path), `line`, `column`. NO user id, NO
  machine id, NO paths, NO endpoints, NO job ids, NO credentials. A Rust
  harness test parses the charter's table (CARGO_MANIFEST_DIR-relative) and
  requires EXACT per-kind equality with the code's allowed keys; the TS gate
  parses the same table and requires equality with the TS mirror the Settings
  UI renders. Both sides pivot on the WRITTEN document.
- **D3 — consent is opt-out, enforced at the single write point.**
  Default = collected (opt-out per the lane definition). The switch lives in
  Settings → Privacy; the only writer is `Telemetry::set_consent` (0600,
  atomic); `record` checks consent at the write point — off = the event is
  dropped before any file IO. Missing/corrupt consent file = the default
  (never a startup panic — the vault's honesty pattern).
- **D4 — the buffer is local, capped, inspectable, clearable.**
  `telemetry.jsonl` in the app-data dir, one JSON object per line, capped at
  500 records (rotate keeping the newest 250 — bounded disk, the #72 budget
  spirit). Settings shows the count, the absolute path, and a two-step-clear
  button (the remove-credentials reflex). No transport code exists in this
  build; when the sink lands it must gate on consent AND a configured endpoint
  AND a one-time disclosure — a follow-up issue, not this lane.
- **D5 — observation points: the watcher's terminal transition + the panic hook.**
  The watcher records `job_terminal` from the final snapshot's non-user fields
  only. `setup` installs a panic hook that records the panic event and CHAINS
  to the default hook (the real panic still prints). A process `OnceLock` lets
  the hook and the commands reach the same instance; unset = drop silently.
- **D6 — the honest limit is written, not hidden.**
  A panic message can name a file the engine touched. Every crash reporter
  ships this trade (Sentry/Chrome/Firefox included); IUM's answer is the
  big-tech one plus one better: verbatim payload, the stance documents it, the
  user can READ every buffered event before any transport could ever exist,
  and the master switch turns the whole thing off.
- **D7 — the product surface: Settings → Privacy, from the #67 primitives.**
  The Privacy card leads the dialog: the stance sentence, the master switch
  (labeled checkbox), the what-we-collect lines (rendered from `TELEMETRY_COPY`
  in copy.ts), the buffer line + Clear. New IPC: `telemetry_status` /
  `telemetry_set_consent` / `telemetry_clear`. `TELEMETRY_COPY` joins the brand
  gate's enumerated surface (W6). The a11y harness already audits this dialog —
  the card must keep it green. e2e seams untouched (the dialog is not
  e2e-driven; no inventory changes).

## RED (captured before implementation, at HEAD)

- TS: `ium-app/src/telemetry.test.ts` — charter exists, required sections, the
  table equals `TELEMETRY_FIELDS`; at HEAD the charter is absent (ENOENT) and
  `TELEMETRY_FIELDS`/`TELEMETRY_COPY` are not exported (import failure).
- Rust: `ium-app/src-tauri/src/telemetry.rs` in gate-only form — the charter
  drift test compiles standalone (pure std) and fails on the missing charter
  ("the charter exists" assertion), the lane-6 ENOENT shape.

## Gates

Rust harness (pure-std, no externs): consent default + set + persist; consent
off = no file IO; allowlist fields exactly (construction + drift test); JSON
escaping (quotes/backslashes/control chars); buckets; cap/rotate; clear;
corrupt consent = default. TS: vitest (telemetry + brand voice over the new
surface), tsc strict, build; axe stays green on the settings dialog. Shell
compile proof = CI (no GTK, the standing contract); core untouched; gate.sh on
the committed tree. One commit, `Closes #75`; STATUS.md evidence row; CHANGELOG
[Unreleased] entry.
