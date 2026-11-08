### CI RED — the launch window's first WebDriver verdict (run 42441459949)

The push of `ca8d04b` gave the e2e lane its first look at the launch window.
7/8 checks green (fmt, core ×3 OS, ium-app ×3 OS — the palette and the shell
compile clean everywhere), but **app-e2e RED** — the lane that had been
green on every run of the previous week:

```
thread 'shell_drives_create_progress_pause_resume_done_inspect_and_cancel'
  panicked at ium-e2e/tests/e2e_shell.rs:532:34:
clear connections: "wd 400 Bad Request element not interactable:
  (POST /element/node-BE3A37E2-…/clear {})"
```

**Wire facts (log job 128611379156):**

- W1 — the boot dump shows the WebDriver attached to the main document
  (`title=IUM — Internet Upload Manager`); session creation consumed ~35 s
  of cold GTK boot; the interaction burst is the last ~1 s.
- W2 — `reveal_main` is idempotent and shared by the handoff AND the 8 s
  shell watchdog; at interaction time (t≈35 s) the watchdog had long fired,
  so the splash was closed and the main window shown + focused.
- W3 — the two native `send_keys` calls (`#upload-path`, `#upload-endpoint`)
  SUCCEEDED moments before — the window and its webview were interactable.
- W4 — only `#upload-connections`'s `clear` was refused (400, not
  interactable). The form markup is byte-identical to the green runs
  (`#65`-pinned ids); the delta set of the red run is exactly {the splash
  window exists, the main window is born hidden, the Ink fonts/13 px}.

**Honest reading:** the micro-cause cannot be reproduced locally (the
sandbox has no WebKitGTK — this lane's oracle is CI, the #61 lesson). Two
candidate mechanisms survive W1–W4: a transient WebKit interactability
refusal at the freshly-revealed window under xvfb, or an in-page
click-point obscuring that only manifests with the new type metrics. The
fix does not have to guess between them.

### Fix design (the loop: make the handoff mechanical, make the next RED speak)

F1 — **The handoff marker.** `reveal_main` already owns the reveal; it now
declares it in the page itself: `main.eval("document.documentElement.dataset.revealed = '1'")`.
The e2e then waits for `html[data-revealed='1']` (30 s budget) before its
first interaction — the #87 acceptance sentence "the WebDriver lane drives
the main window after handoff" becomes a mechanical wait, not an
assumption. The e2e wait IS the marker's pin (the shell lane cannot drive
a window without a display; the WebDriver lane is its contract holder).

F2 — **Interactability is now an explicit poll.** `send_keys`/`clear` gain
the same doctrine `wait_for_element` has always had: a 400 "not
interactable" is retried inside a bounded budget (10 s), and a final
failure carries FORENSICS — the element's box, displayed/disabled state,
and `document.elementFromPoint` at the click point — so if this ever
recurs, the log names the culprit instead of guessing.

F3 — **No app-behavior change.** The marker is a dataset attribute; the
splash, the handoff, the watchdog and the visible UI are untouched. Zero
new dependencies. `click` stays bare (no observed defect — no drive-by).

RED = the CI run above (the failing test already exists — the lane itself).
Green = CI re-run on the fix commit, the only WebKit oracle.
