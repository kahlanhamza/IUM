RED evidence — #93 (terminal notifications), at HEAD `059d76e`

The pins (`ium-app/src/notify.test.ts` — 11 behavior pins: the transition
matrix, the copy matrix, idempotence, the focus rule, the basename rule, the
persisted pref) cannot even resolve their module at HEAD:

```
 FAIL  src/notify.test.ts [ src/notify.test.ts ]
Error: Failed to resolve import "./notify" from "src/notify.test.ts". Does the file exist?
 Test Files  1 failed (1)
      Tests  no tests
```

No notification code exists anywhere in the shell (grep: zero `notification`
hits in ium-app). Implementation next; the copy matrix lands in the
enumerated voice surface (`copy.ts`), so the brand gate keeps its teeth.
