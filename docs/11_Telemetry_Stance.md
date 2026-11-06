# The IUM telemetry stance

The written privacy contract (product bar #66 lane 7, issue #75). This
document is the pivot: a Rust harness test and a vitest gate parse the
collected-fields table below and require it to equal the code's allowed
fields EXACTLY — the stance cannot drift from the implementation, and the
implementation cannot grow a field without rewriting the stance first.

## The decision

IUM ships **anonymous diagnostics, opt-out** — #66's option (a), decided by
the owner on 2026-11-07 ("we always follow big tech teams behaviors and
architecture so we must ship a real big tech products that users trust").
This is the standard the big products converged on: collection is ON by
default, the switch is first-class and visible in Settings, the disclosure
is written in plain sentences, and the user can inspect and clear every
byte that was recorded. The engine is already measured everywhere a drill
can reach; the product finally gets the same honesty about itself.

IUM collects anonymous diagnostics to keep uploads honest — the record
stays on the user's machine, and one switch turns it off.

## What IUM collects

Exactly two event kinds, carrying exactly these fields — nothing else:

| field | event | example |
|---|---|---|
| `v` | every | `1` |
| `kind` | every | `job_terminal` |
| `ts` | every | `1730900000` |
| `adapter` | job_terminal | `tus` |
| `state` | job_terminal | `done` |
| `size_bucket` | job_terminal | `1_16mib` |
| `duration_bucket` | job_terminal | `10_60s` |
| `message` | panic | `bounds exceeded` |
| `source` | panic | `ium-core/src/chunk.rs` |
| `line` | panic | `412` |
| `column` | panic | `9` |

- `job_terminal` — one event when an upload reaches a terminal state
  (done / failed / unresumable / canceled): the transfer method, how it
  ended, and the size and duration as WIDE buckets (5 size buckets, 6
  duration buckets — the width is the anonymity).
- `panic` — one event when IUM itself panics: the payload and the panic
  location inside IUM's own source.

## What IUM never collects

File names, folder paths, destination endpoints, URLs, credentials,
account ids, job ids, machine identifiers, install identifiers, file
contents. The event builder reads the terminal snapshot's NON-user fields
only — the journal's `source_path` and `endpoint` (plain text by design)
are structurally unable to reach an event. There is no identifier of any
kind in any event.

## Where it lands

`telemetry.jsonl` in the OS app-data directory (the journal's directory),
one JSON object per line, capped at 500 records (rotation keeps the
newest 250). **No transport ships in this build** — there is no HTTP
client in the shell, no endpoint, nothing: the record is a local file the
user can open, read, and clear from Settings → Privacy. Nothing leaves
the device by construction, not by promise.

The upload sink (a Sentry DSN or a self-hosted collector) is the PINNED
OWNER ASK — the same pattern as the release-signing key (lane 4 shipped
the full pipeline and pinned the credential). When a sink ships it must
gate on all three of: the consent switch AND a configured endpoint AND a
one-time disclosure naming the destination. Until then, this section is
the whole truth: the data lands nowhere but the local file.

## The switches

Settings → Privacy carries the product surface:

- **Collect anonymous diagnostics** — the master switch, ON by default
  (opt-out), enforced at the single write point: off means the event is
  dropped before any file IO happens.
- **Clear local events** — a two-step confirm that empties the buffer.
- The buffer's absolute path, shown so the file is one click away.

The consent choice persists (`telemetry_consent.json`, 0600, atomic
write); a missing or corrupt consent file falls back to the default
without ever blocking startup.

## The honest limits

A panic message can name a file the engine touched at the moment of the
crash — that is true of every crash reporter in the industry (the
Sentry/Chrome/Firefox class included), and IUM ships the trade in the
open instead of pretending it away: the payload is recorded verbatim, the
charter says so, the user can read every buffered event in a text editor
before any transport could ever exist, and the master switch turns the
whole pipeline off. Everything else about an event is bucketed beyond
re-identification.

## The gates

- **Rust** (`ium-app/src-tauri/src/telemetry.rs`, pure-std standalone
  harness): parses this file's table and requires per-kind equality with
  the code's allowed fields; proves the consent default, the off switch
  causing zero file IO, the allowlist by construction, JSON escaping,
  bucket widths, the cap/rotate, and clear.
- **TS** (`ium-app/src/telemetry.test.ts`): this document exists with its
  sections; the table equals `TELEMETRY_FIELDS` — the mirror the Settings
  UI is built from.
- **Voice** (`ium-app/src/brand.test.ts`): the Privacy surface's sentences
  ride the enumerated copy table — capitalized, calm, no exclamation
  marks, no emoji, the name spelled IUM.

## Changing the stance

The table above may only change together with the code: add the field to
the charter AND to both gates' constants in the same reviewed diff, or
the build fails. The consent default (opt-out) is part of the recorded
owner decision and changes only by a new decision, never by a refactor.
