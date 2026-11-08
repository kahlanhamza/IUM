### GREEN — CI verdict 8/8 on the landing commit `e000de7` (run 42446590112)

The anatomy pass closed end to end, first WebKit verdict green:

| step | receipt |
| --- | --- |
| Design note from the wire facts | comment 6716987995 (the table stays; the ONE collision — mockup chunk grid vs the e2e's chunk table — moved by the D4 same-commit rule) |
| RED at HEAD | comment 6717012103 — 8 anatomy pins + 2 chrome pins failing; the shadow-law and backdrop-filter pins green at HEAD and kept green (law retention) |
| The swap | `e000de7`: four-pane anatomy at the mockup's metrics, docked inspector + chunk grid, modal settings, drawer retired, the shared theme store, the e2e chunk leg + locator doc amended in the same commit |
| Local gates | vitest **129/129** (16 files, was 120) · tsc strict · build 208.13 KB JS / 64.71 KB gzip · WCAG AA both schemes (audit clean) · fmt clean · `ium-e2e` compiles · `gate.sh` GREEN on the committed tree |
| CI | run 42446590112 **8/8 GREEN** — fmt, core ×3 OS (313/313), ium-app ×3 OS (129/129), **app-e2e on WebKitGTK/xvfb driving the NEW anatomy**: the docked pane, the chunk grid (`data-idx`/`data-state`/`data-confirmed`, Σ == source size), the create panel, the pause/resume legs — all first-try green |
| Receipts | the six README screenshots re-shot through the real harness — they show the anatomy; the README itself updated (the Ink face in Highlights, the connect lanes in Transports, the roadmap un-staled) |

The Ink program is complete: #86 the palette, #87 the launch window, **#88 the
chrome anatomy** — the owner-approved mockups are now the app, pinned by tests
on all three OSes.
