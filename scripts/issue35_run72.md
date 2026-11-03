## Run #72 evidence + harness fix (row click)

**Leg A is fully green through the real shell now** (#39 + #40 fixes landed):
run #72's recorder record shows `tus_sessions: 1, tus_bytes: 25165824` — the full
24 MiB again — and the test passed the running-wait, the live-progress deadline
poll, pause, resume and the Done badge. It failed one step later:

```
ium-e2e/tests/e2e_shell.rs:565:10:
click row A opens inspector: "wd 400 Bad Request element not interactable:
(POST /element/node-0F38BC74-…/click {})"
```

**Mechanism:** the jobs table is wider than the window (7 columns; the Actions
column carries three buttons), so the `<tr>` is a full-viewport-plus-width
element — a row cannot be scrolled into view horizontally and the click atom
refuses it ("element not interactable"; geckodriver#1294 is the same mechanism
class: "Element <tr> could not be scrolled into view — clicking on row, wider
than screen, failed"). The row's own action buttons clicked fine all along
because a small button scrolls into view individually.

**Fix (harness-only, same user path):** click the row's first CELL — the onClick
lives on the `<tr>` and every cell bubbles to it; a user clicks a cell too. No
product change, no seam: the inspector still opens through a real DOM click on
the row.

The failure chain so far, each face killed in turn: #70 abort at the Upload click
(#39, tokio context) → #71 row frozen at `probing pct=0` (#40, ACL granted
nothing) → #72 inspector row click (#35 harness, this fix). Remaining unexercised
steps after the inspector: close drawer, server-truth HEAD, leg B one-shot
create/cancel/404.
