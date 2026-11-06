# Issue #75 (Lane 7) — RED evidence (captured at HEAD c1e926e, 2026-11-06)

## RED — the stance is not WRITTEN and nothing guards it

**Rust** (`CARGO_MANIFEST_DIR=…/src-tauri rustc --test telemetry.rs --edition 2021` —
pure-std, gate-only form, no product code yet):

```
test charter_table_equals_code_allowlist ... FAILED
thread 'charter_table_equals_code_allowlist' panicked at telemetry.rs:46:6:
the charter exists: Os { code: 2, kind: NotFound, message: "No such file or directory" }
test result: FAILED. 0 passed; 1 failed
```

**TS** (`npx vitest run src/telemetry.test.ts`):

```
FAIL  src/telemetry.test.ts > the telemetry charter — the field table cannot
drift (D2) > the charter's collected-fields table equals TELEMETRY_FIELDS exactly
 Test Files  1 failed (1)
      Tests  4 failed (4)
```

All four TS contracts fail on the same root cause — `docs/11_Telemetry_Stance.md`
is absent at HEAD (the #66 acceptance bar "The telemetry stance is WRITTEN and
shipped accordingly" has no artifact and no gate) and `TELEMETRY_FIELDS` /
`TELEMETRY_COPY` are not exported from `copy.ts` (the UI mirror does not exist).
The Rust gate fails on the same missing artifact. The charter is the pivot: after
this lane, neither the document nor the code may move without the other.
