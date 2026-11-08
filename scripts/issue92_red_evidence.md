RED evidence — #92 (materials), re-reproduced FRESH at the pre-lane tree

The lane commit is `44cd024` (implementation + pins in one commit-first tree,
per gate law). To prove the pins bite, the two `data-material` verdict pins
from `ium-app/src/theme.test.tsx` at `44cd024` were run against the pre-lane
`theme.ts` (checked out from the parent `97e2260`, where
`applyPlatformMaterial` does not exist — `grep -c` = 0):

```
 FAIL  src/theme.test.tsx > the materials verdict (#92) > a true verdict sets data-material=mica on the root, pre-paint
 FAIL  src/theme.test.tsx > the materials verdict (#92) > a refused verdict removes it — the window stays exactly as opaque as before
TypeError: applyPlatformMaterial is not a function
 ❯ src/theme.test.tsx:134:5
 Test Files  1 failed (1)
      Tests  2 failed | 10 passed (12)
```

The 10 pre-existing theme pins keep passing at the parent tree — the RED is
exactly the two verdict pins, nothing else. Working tree restored to
`44cd024` immediately after (verified clean). Gates on the committed tree
next.
