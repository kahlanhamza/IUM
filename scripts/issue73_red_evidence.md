# Issue #73 (Lane 6) — RED evidence (captured at HEAD 61f3764, 2026-11-06)

## RED — the brand sheet does not exist, nothing guards the acceptance bar

`npx vitest run src/brand.test.ts` at current HEAD:

```
 FAIL  src/brand.test.ts > the brand sheet — the token projection cannot drift (D2) > the sheet's hex values equal theme.css's actual values — light and dark
Error: ENOENT: no such file or directory, open '/home/z/my-project/docs/10_Brand_Sheet.md'
 ❯ src/brand.test.ts:99:30

 Test Files  1 failed (1)
      Tests  4 failed (4)
```

All four contracts fail on the same root cause — `docs/10_Brand_Sheet.md` is
absent at HEAD (W5): the #66 acceptance bar ("a brand sheet exists in-repo
and UI copy follows it") has no enforcing artifact and no gate. The test
file `ium-app/src/brand.test.ts` defines the contract: existence + required
sections, the token projection equality with theme.css (D2), and the
mechanical voice rules over the enumerated copy surface (D3, second stage).
