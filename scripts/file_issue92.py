#!/usr/bin/env python3
"""File issue #92: materials — Windows 11 Mica, build-gated (P3)."""
import json
import urllib.request

with open("/home/z/my-project/.secrets/gh_token") as f:
    token = f.read().strip()

body = """## Design note (materials — the third owner-ordered lane)

The approved platform plan: macOS carries its native traffic lights, Windows 11
gets **Mica** (compositor-side — the cheap material: one blurred wallpaper tint
behind the window, GPU-composited, no per-frame cost), Linux stays DE-dependent
(no material). The splash is untouched — the identity moment stays opaque.

### The verdict is an OS fact, not an API opinion

Wire fact (tauri-apps/tauri report, 2026-06-04): on Windows 10 `set_effects`
can return **success while the window renders fully transparent** — the effect
call's Ok proves nothing. So the gate reads the **OS build**: Mica exists on
Windows 11 (build ≥ 22000). The shell answers one command:

- `platform_material() -> bool` — Windows: parse the real build from the OS
  (`cmd /c ver`, dependency-free); ≥ 22000 → apply `set_effects([Mica])` and
  return its Ok; elsewhere → `false` without touching the window.
- The frontend applies the verdict pre-paint (`main.tsx` → `theme.ts`):
  `data-material="mica"` lands on `<html>` before React mounts — no flash.

### The CSS gate (only when the verdict is true)

`[data-material="mica"]` makes the window backing transparent and gives the two
chrome tokens translucency — light: bg α 0.72, surface α 0.84; dark: 0.70/0.82.
Text tokens stay OPAQUE ink (the material shows behind chrome and content, the
type never sits directly on the wallpaper). Every other platform (and Win10,
and any failed effect) sees exactly today's opaque window — zero-risk default.

### The audit keeps its teeth

`ink_contrast_audit.py` gains the composited scheme: effective backgrounds =
α·token + (1−α)·(theme base) chained properly (surface composites over bg,
accent_subtle over surface), then the SAME PAIRS table must pass AA on the
composited values. The conservative Mica base approximation is the theme's own
solid `--bg` — if a pair passes there it passes under any realistic tint.

### Verification honesty

The sandbox has no Windows: the compositor verdict itself lands when the owner
runs the next Windows build. What IS verified here: the build parser (unit
tests on the ver string), the verdict wiring (theme pins), the audit's
composited AA (script run in-gate), the e2e neutrality (Linux returns false →
`data-material` absent → the WebDriver lane sees the unchanged opaque shell).

## Acceptance

- [ ] `parse_windows_build` unit-tested (22000+, 19045, garbage).
- [ ] `platform_material` wired + registered; Linux/macOS answer false.
- [ ] `applyPlatformMaterial` pinned (sets and removes `data-material`).
- [ ] Composited AA clean both schemes (audit extended, run green).
- [ ] e2e neutrality: no pinned seam touched; CI 8/8.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps(
        {
            "title": "Materials — Windows 11 Mica behind the Ink chrome, build-gated (P3)",
            "body": body,
            "labels": ["app", "design"],
        }
    ).encode(),
    headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
    },
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    out = json.load(resp)
print(f"filed issue #{out['number']} url={out['html_url']}")
