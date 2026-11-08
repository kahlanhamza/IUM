### GREEN — CI verdict 8/8 on the fix commit `99996e1` (run 42442755726)

The arc of this lane, end to end:

| step | receipt |
| --- | --- |
| Palette + launch window land locally (full loops, RED first) | `22e204e`, `ca8d04b` |
| The PAT returns; RED re-reproduced FRESH against the pre-Ink/pre-launch trees | comments above (9 failed pins + unresolvable `theme.ts`; unresolvable `Launch` + 2 window-contract failures) |
| Gates re-proven in the new sandbox on the committed tree | `gate.sh` GREEN on `ca8d04b` — vitest 120/120 (15 files), tsc strict, vite build, core 313/313, fmt + clippy `-D warnings`, doc lane |
| Push + first CI verdict | run 42441459949: **7/8 — app-e2e RED** (`element not interactable`, the launch window's first WebDriver verdict) |
| The fix (design note above) | `99996e1`: the handoff marker (`data-revealed="1"` set by `reveal_main`, waited for mechanically by the e2e) + interactability as an explicit bounded poll with page-side forensics on final refusal |
| Final CI verdict | **run 42442755726: 8/8 GREEN** — fmt, core ×3 OS (313/313), ium-app ×3 OS (vitest 120/120, tsc, build), **app-e2e green on WebKitGTK under xvfb** |

The launch window now has its full proof chain: component pins (the handoff
fires exactly once, both paths), the config contract (windows declared in
the designed order), and the WebDriver lane driving the real shell after
the handoff — on the only WebKit oracle this repo has.

Closing as completed. The Ink program's remaining lane is **#88** (the
chrome anatomy pass).
