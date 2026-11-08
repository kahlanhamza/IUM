### RED — reproduced fresh at the pre-Ink tree (2026-11-08, this sandbox)

The PAT returned and the RED was re-proven before anything shipped: the two
pin files from the landing commit (`brand.test.ts`, `theme.test.tsx`) were
run against the parent tree `e090504` (Ink implementations absent). Verbatim:

```
 ❯ src/brand.test.ts (17 tests | 9 failed) 32ms
 Test Files  2 failed (2)
      Tests  9 failed | 8 passed (17)
```

The 9 failing pins in `brand.test.ts`:

```
FAIL  the brand sheet (#73) — existence and sections > carries every required section
FAIL  the brand sheet — the token projection cannot drift (D2) > the gate survives a windows-default checkout (the #74 simulation: CRLF disk bytes)
FAIL  the brand sheet — the token projection cannot drift (D2) > the sheet's hex values equal theme.css's actual values — light and dark
FAIL  the brand sheet — the logo scale ladder (#86) > pins every rung of the logo ladder at its exact size
FAIL  the brand sheet — the logo scale ladder (#86) > carries the ladder rule as a sentence (identity surfaces go bigger)
FAIL  the Ink type system (#86) — vendored Inter + JetBrains Mono > theme.css declares every brand face as @font-face over the vendored files
FAIL  the Ink type system (#86) — vendored Inter + JetBrains Mono > the vendored woff2 files exist on disk and are real (not placeholders)
FAIL  the Ink type system (#86) — vendored Inter + JetBrains Mono > the UI and mono stacks lead with the brand faces, OS faces stay as fallback
FAIL  the Ink theme attribute (#86) — both modes are first-class > the dark scheme is an attribute block, not only a media query
```

sample assertion (the dark attribute block):

```
 ❯ src/brand.test.ts:240:17
    238|   it("the dark scheme is an attribute block, not only a media query", …
    239|     const css = readTheme();
    240|     expect(css).toContain('[data-theme="dark"]');
        |                 ^
```

and `theme.test.tsx` fails to collect wholesale — the theme module itself
does not exist at the base:

```
FAIL  src/theme.test.tsx [ src/theme.test.tsx ]
```

(The commit message counted 8 distinct pins; the fresh vitest count is
9 failing cases because the two D2 drift guards plus the #73 sections test
fail as a consequence of the same absent facts. Both numbers describe the
same RED.)

The swap then landed as `22e204e`: all pins green, suite 114/114 at the
time, 120/120 after #87 joined.
