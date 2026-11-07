# RED evidence — #77 (at HEAD 3857b93)

```
npx vitest run src/header.test.tsx src/readme.test.ts
 Test Files  2 failed (2)
      Tests  6 failed (6)

FAIL src/header.test.tsx
  × the quiet header (#77) > never renders the engine's raw sentence, a version, or the adapter roster
    → found element with text "Connected"
    (at HEAD the pill renders engine.detail verbatim: "ium-core v0.1.1 ready — adapters: http_one_shot, s3_multipart, tus")
  × the quiet header (#77) > the brand block is the constructed logomark — real SVG paths, not the generic icon
    → header renders a .logomark
FAIL src/readme.test.ts
  × embeds every committed product screenshot      → none of the six paths appear in README.md
  × the screenshots section exists before the architecture section → "## Screenshots" missing
  × no stale roadmap claims — the tree has moved past v0.1.0 / M0–M7
    → README still says "M0–M7 are complete … release packaging toward v0.1.0"
  × the product's trust surfaces are stated — telemetry stance and honesty
    → no telemetry/Privacy sentence in README
```

The failing output above is the design debt the owner called out, pinned as tests:
the header leaks the engine's debug sentence, no logomark exists, the README has
no screenshots and a stale roadmap. Green comes with the design pass in this issue.
