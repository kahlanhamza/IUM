## Run #74 evidence + the recorder's startup record

**Leg B ran and was canceled honestly** — the content-addressed row fix worked:
the test passed `job B running`, clicked Cancel, and the `Canceled` badge assert
passed. It died at the very last server-truth read:

```
ium-e2e/tests/e2e_shell.rs:740:50:
record B: "record file /tmp/ium-e2e.Miq1RL/recorder-b.json never appeared"
```

**Mechanism:** the cancel leg fires the moment `running` shows — and with the
one-shot family's confirmed-bytes unable to move mid-flight, that can be BEFORE
the PUT's first byte (the drive loop journals `Running` when it starts leasing;
the connection opens microseconds to seconds later). The recorder honestly saw
ZERO requests — and it only ever wrote its record file **on the first request**,
so the e2e's `read_record` (10 s poll) reported "never appeared": a perfect
"holds nothing" ending mistaken for a missing server.

**Fix (tooling, honest by construction):** the loopback recorder now writes its
record file **at boot** — an all-zero record IS the truth "this recorder has
served nothing yet". Smoke-tested locally: record exists at startup
(`{"put_count": 0, "received_bytes": 0, "tus_sessions": 0, "tus_bytes": 0,
"tus_last": null}`), HEAD before any PUT answers 404, a PUT updates the record.
`read_record` stays strict (a missing file is still a real failure — recorder
boot is already asserted by app-e2e.sh's `listening` grep).

The cancel leg's pinned semantics are unchanged and now properly exercised:
whatever the timing — cancel before the first byte (record stays all-zero) or
mid-body (the read error commits nothing) — the recorder holds NOTHING and HEAD
answers 404. Both paths are the honest "nothing committed" ending.

Failure chain: #70 abort at click (#39) → #71 frozen row (#40) → #72 row click
(cell) → #73 wrong-row watch (content-addressed rows) → #74 record-at-boot
(this fix). After this, every step of both legs has executed at least once.
