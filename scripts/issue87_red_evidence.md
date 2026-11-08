### RED — reproduced fresh at the pre-launch tree (2026-11-08, this sandbox)

The launch pins from the landing commit (`launch.test.tsx`,
`launch.config.test.ts`) were run against the parent tree `22e204e` (no
splash window, no `Launch.tsx`). Verbatim:

```
Error: Failed to resolve import "./components/Launch" from "src/launch.test.tsx". Does the file exist?
 FAIL  src/launch.test.tsx [ src/launch.test.tsx ]

Failed Tests 2
FAIL  src/launch.config.test.ts > the window contract (#87) > declares main first and hidden (handle 0 stays the e2e seam)
AssertionError: expected 1 to be greater than or equal to 2

FAIL  src/launch.config.test.ts > the window contract (#87) > declares the splash: frameless, centered, on top, non-resizable, routed
TypeError: Cannot read properties of undefined (reading 'label')

 Test Files  2 failed (2)
      Tests  2 failed (2)
```

Read: `tauri.conf.json` declared a single window (hence `expected 1 to be
>= 2`) and no `splash` window existed (hence `undefined (reading 'label')`);
the `Launch` component did not exist. Exactly the three facts the lane set
out to fix.

The swap then landed as `ca8d04b`: the window contract green, the lifecycle
bug the suite caught (the handoff timer not cleanup-owned) fixed pre-landing,
3 consecutive full-suite runs 120/120.
