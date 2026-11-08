GREEN evidence — #92 (materials), landed across three commits

**The lane:** `44cd024` (the verdict + the CSS gate + the audit) → `9d54534`
(run-184 fixes: public `WindowEffect` path, platform-free verdict
expression) → `0abca16` (run-185 fix: the parser's dead-code silence scoped
to non-Windows lib targets). **CI run 186 (42511754427): 8/8 success** —
core ×3 OS, **ium-app shell lane ×3 OS** (ubuntu/macos/windows — the lane
that caught both defects), fmt, app-e2e on WebKitGTK.

## RED → GREEN

- Issue RED (comment 6725196893): the two `data-material` verdict pins run
  against the pre-lane `theme.ts` — `TypeError: applyPlatformMaterial is
  not a function`, 2 failed | 10 passed; the 10 pre-existing pins keep
  passing at the parent tree.
- CI RED #1 (run 184, comment 6725385150): 4 lib errors + E0603 on Windows
  (private `WindowEffect` re-import — Windows could not compile at all).
- CI RED #2 (run 185): one residual dead-code on non-Windows lib targets.
- GREEN: gates on every committed tree (vitest **146/146** across 17 files,
  tsc strict, build, core fmt/clippy/tests); CI **8/8 on run 186**; e2e
  neutrality held (Linux answers false → `data-material` absent → the
  WebDriver lane drove the unchanged opaque shell).

## What shipped

- `lib.rs`: `parse_windows_build` (platform-free, unit-tested on every
  runner), `real_windows_build` (`cmd /c ver`, dependency-free),
  `platform_material` = `real_windows_build() >= 22000 && apply_mica(&app)`;
  `apply_mica` two cfg variants — Windows 11 applies
  `set_effects(WindowEffectsConfig { effects: vec![Mica], ..Default::default() })`
  through the PUBLIC `tauri::utils::WindowEffect` path; every other platform
  ignores the handle and answers false. Registered in `generate_handler`.
- `tauri.conf.json` transparent main window; `theme.ts::applyPlatformMaterial`
  pre-paint (`data-material` set before React mounts); `theme.css` chrome
  tokens translucent only under the verdict, ink text tokens stay opaque.
- `ink_contrast_audit.py`: the composited Mica scheme joins the audit —
  effective backgrounds chained (surface over bg, accent_subtle over
  surface), same AA pairs table, clean all four schemes.

## Verification honesty (unchanged)

The sandbox has no Windows: the compositor verdict itself lands when the
owner runs the next Windows build. What CI proved today: the lane compiles
and lints clean on all three OS, the Linux verdict is honestly false, and
the e2e seam is untouched. Closing completed.
