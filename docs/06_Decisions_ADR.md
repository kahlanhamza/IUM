# IUM — Architecture Decision Records

Lightweight ADR log: one entry per decision, status, and the consequence we accept.
Ratified retroactively where marked.

## ADR-001 — Language & runtime: Rust + tokio, reqwest with rustls ✅ ratified (Sprint 0)
Crash-safety, streaming bodies, no OpenSSL cross-platform pain. Consequence: slower iteration
than GC languages; accepted because correctness is the product.

## ADR-002 — Journal: SQLite (WAL) via rusqlite, journal is the single source of truth ✅ ratified
Versioned migrations via `PRAGMA user_version`; job+chunk inserts transactional; every write
checks rows-affected and fails loudly; every state transition is journaled **before** it
executes; on boot the manager rebuilds reality from the journal. Consequence: all state reads
go through the journal — caller arguments are never trusted.

## ADR-003 — Chunk pool: dynamic segments with leases ✅ ratified
Files split into dynamic chunks; workers lease chunks; a partial confirm returns the chunk to
the rentable pool keeping its confirmed offset. Consequence: the premature-Done bug class is
structurally excluded, plus a `finish()` consistency gate as belt-and-suspenders.

## ADR-004 — Verify-before-declare ✅ ratified (hardening open, see Roadmap M2)
A job may only reach `Done` after the adapter verifies the upload (HTTP adapter: HEAD
Content-Length). **Open gap:** `TransferAdapter::verify` currently has a default "pass" impl —
M2 makes it a required method so no future adapter can inherit silence.

## ADR-005 — Slow-connection eviction: throughput watch + strikes ✅ ratified (2026-11-02)
Per-lease sliding-window throughput; a worker below `floor_ratio × median(pool)` for
`strikes_to_evict` consecutive windows is evicted (aria2's pattern, rebuilt for uploads, spec
§4.2). Pure decision logic, unit-tested without real clocks. Lone-worker and fully-idle pools
never evict (no thrash).

## ADR-006 — Engine first, UI frozen ✅ ratified (team directive)
`ium-app` (Tauri 2 + React) does not join the workspace and is not compiled in CI until M2+M3.
Consequence: no demo UI until the engine is trustworthy — deliberate.

## ADR-007 — Gates are non-bypassable ✅ ratified (2026-11-02)
No commit lands without `fmt --check` + `clippy -D warnings` + full test suite green locally.
Evidence: one ungated snapshot commit (`0c11211`) introduced a clippy hard error, a test that
could never pass, and fmt drift — all caught within one day once the gates were actually run.

## ADR-008 — STATUS.md is the single source of truth, pushed every task ✅ ratified (2026-11-02)
Team decision: sandbox recycles wipe context; GitHub holds the memory. Every task ends with
STATUS.md updated and pushed. A task that is not pushed does not exist.

## ADR-009 — Clean-room policy ✅ ratified
Vendor behavior is studied by observing printable strings and public documentation only
(`scripts/elf_strings_scan.py`, `scripts/idm_pe_scan.py`, `scripts/magic_carver.py`; evidence in
`rea-lab/out/`). No disassembly, no decompilation, no vendor code or assets in the repo.
