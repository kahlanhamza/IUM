CI run 184 (44cd024) — the oracle caught what the local gate cannot reach. The
sandbox has no GTK dev libs, so the `ium-app` **Rust** lane is CI-only (the
local gate says so explicitly); all four `ium-app` shell-lane jobs failed.
Full log enumeration:

**Linux/macOS (4 lib errors, 3 lib-test errors):**
1. `unused variable: app` — `platform_material`'s handle is only read inside
   the `#[cfg(windows)]` block; on every other platform the param is dead.
2. `parse_windows_build` never used (dead_code) on the non-test target.
3. `real_windows_build` (the `not(windows)` stub) never called — its only
   call site sits inside the same windows-only block.
4. clippy `manual_pattern_char_comparison` on
   `split(|c: char| matches!(c, '[' | ']' | '\r' | '\n'))`.

**Windows (worse — E0603, does not compile at all):**
`tauri::utils::config::WindowEffect` is a PRIVATE re-import in tauri-utils
2.10.2 (`use crate::{…, WindowEffect, …}` at config.rs:64); the public,
`#[non_exhaustive]` enum lives at the crate root. `set_effects` itself is
verified good: `WebviewWindow::set_effects<E: Into<Option<WindowEffectsConfig>>>
-> Result<()>`, desktop-only; `WindowEffectsConfig` is public in
`tauri_utils::config` and derives `Default`.

**Fix plan (one commit, no behavior change):** the verdict keeps its shape —
OS-build gate first (`real_windows_build() >= 22000 && apply_mica(&app)`),
so the helpers are called on every platform (kills 1–3); `apply_mica` gets
two cfg variants — Windows applies `set_effects` with the PUBLIC path
`tauri::utils::WindowEffect::Mica` and a struct literal
(`..Default::default()`), every other platform ignores the handle
(`_app`) and answers false; the split predicate becomes a char-array
pattern (kills 4). Behavior is byte-identical: Windows 11 still gates on
build ≥ 22000 + a successful Mica apply; everything else stays opaque.
