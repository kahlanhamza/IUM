## RED at HEAD (the distro pins fail, the trust root holds)

`npx vitest run src/distro.test.ts` on clean HEAD `2d0a5f7` (the new
`ium-app/src/distro.test.ts`, the `readme.test.ts` house pattern):

```
 ❯ src/distro.test.ts (3 tests | 2 failed) 10ms
   × the baked updater endpoint is the PUBLIC distro feed
     → expected [ Array(1) ] to include 'https://github.com/kahlanhamza/IUM-di…'
   × no endpoint points at the private source repo
     → the source repo is private — its feed 404s for every user:
       expected [ Array(1) ] to deeply equal []
 Test Files  1 failed (1)
      Tests  2 failed | 1 passed (3)
```

Exactly the designed failure shape:
- the two ENDPOINT pins fail — `tauri.conf.json` still bakes the private
  source repo's feed (the wire-measured 404 for every user);
- the PUBKEY pin **passes** — the minisign trust root is untouched, as the
  design demands (a channel move must never rotate the key silently).

Green comes with the endpoint retarget in the same commit.
