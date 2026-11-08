### RED — the anatomy pins at HEAD (2026-11-08, fresh)

The #88 pins (anatomy.test.tsx — new) and the re-homed quiet-chrome pins
(header.test.tsx — rewritten) run against the pre-anatomy tree. Verbatim:

```
   × the anatomy metrics are the mockup's (#88, from the real stylesheet) > toolbar 48 / sidebar 210 / rows 36 / status bar 28 / inspector 150 docked / modal 640 10ms
   × the anatomy metrics are the mockup's (#88, from the real stylesheet) > the inspector docks — it pushes content, it never overlays (no fixed positioning) 1ms
   × the anatomy metrics are the mockup's (#88, from the real stylesheet) > the drawer is retired — .drawer and .drawer-backdrop are gone from the sheet 2ms
   × the anatomy in the rendered shell (#88) > the toolbar carries NO brand block — the ladder's Chrome rung, enforced in the component 251ms
   × the anatomy in the rendered shell (#88) > the toolbar commands: New Upload focuses the path field; the settings seam rides along 1013ms
   × the anatomy in the rendered shell (#88) > the sidebar library filters the work pane — real filters, real counts 1034ms
   × the anatomy in the rendered shell (#88) > the status bar speaks the SHORT truth — version yes, raw sentence never 6ms
   × parseVersion — the status bar's one piece of presentation math > reads the engine's own sentence, or admits it cannot 0ms
⎯⎯⎯⎯⎯⎯⎯ Failed Tests 8 ⎯⎯⎯⎯⎯⎯⎯
      Tests  8 failed | 2 passed (10)
```

```
AssertionError: .toolbar exists in theme.css: expected -1 to be greater than or equal to 0
AssertionError: .inspector exists in theme.css: expected -1 to be greater than or equal to 0
AssertionError: expected '/*\n * IUM design system — "Ink" (#86…' not to match /^\.drawer \{/m
AssertionError: expected 'app-header' to match /toolbar/
TestingLibraryElementError: Unable to find role="button" and name "New Upload"
TestingLibraryElementError: Unable to find role="button" and name `/All uploads/`
```

```
⎯⎯⎯⎯⎯⎯⎯ Failed Tests 2 ⎯⎯⎯⎯⎯⎯⎯
AssertionError: the status bar carries the one ambient signal: expected null not to be null
AssertionError: no .brand in the working chrome: expected <div class="brand">…(2)</div> to be null
      Tests  2 failed (2)
```

Read: no .toolbar/.sidebar/.statusbar/.inspector/.modal in theme.css; .drawer still
present; the shell renders app-header + brand + no New Upload + no filters + no
status bar; parseVersion does not exist. The shadow-law and backdrop-filter pins
PASS at HEAD — they are law-retention pins and stay green through the swap.
