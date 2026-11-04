## CI run #78 post-mortem → the entry-HEAD fix (commit on main)

Run #78 (the #42+#43 push) failed the **TUS wire tests step on all 3 OS**
while every local gate was green. The failing assertion:
`tus_offset_conflict_realigns_from_server_offset` pins the exact invariant

```
moved() − wasted() == the re-PATCH tail (2 MiB) exactly
```

The #43 reckoning computed waste as `pushed − (final − resume_from)` —
correct ONLY when the server sits at `resume_from` at call entry. The
drill's own setup (server drifted to 3 MiB, journal at 1 MiB) violates
that: the 2 MiB of pre-existing drift were subtracted from THIS call's
pushes, under-counting waste by exactly the drift → `moved − wasted`
read 4 MiB (or flaked when the first flush saturated).

**The fix — HEAD at upload_chunk entry (the alignment point):**
- the server's entry offset starts the dance: when the journal is behind,
  the guaranteed-409 blind PATCH disappears (first PATCH goes to server truth);
- server-persisted during this call = `final − entry` — no ambiguity
  between pre-existing drift and this call's consumed bytes;
- the empty-span (#5) and resplit-drift (#44) confirm-before-send shapes
  fold into the same observation;
- the server-behind-journal guard anchors on the entry offset.

Local proof at the fix commit: full suite 156 passed / 0 failed, and the
real-tusd wire suite (scripts/tus-test.sh, tusd 2.10.1) 12/12 — the exact
lane CI failed. The lesson is recorded: **the entry state must be
observed, never assumed — the same verify-before-declare rule the engine
applies to confirms applies to the waste ledger.**
