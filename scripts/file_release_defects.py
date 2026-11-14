#!/usr/bin/env python3
"""File the v0.2.5 release-defect issues (#113 UI overlap, #114 translucent
modal, #115 console window + mangled OAuth handoff). Idempotent-ish: run once.
Reads the PAT from .secrets/gh_token (never printed)."""
import json
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
MILESTONE = 10  # M11 — the shipping bar: installers, zero-touch, big-tech UX


def api(path, payload=None):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        data=json.dumps(payload).encode() if payload else None,
        method="POST" if payload else "GET",
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


ISSUES = [
    {
        "title": "Connect sheets render broken: field labels paint over each other, inputs crammed (owner screenshot 2026-11-14)",
        "labels": ["bug", "ium-app", "ux", "product"],
        "body": """## The defect (owner evidence, v0.2.4 on Windows)

The S3 multipart connect sheet renders UNREADABLE: the four field labels paint on top of each other ("Bucket URL for the Test button (e.g. https://…)" lands on top of "Access key" / "Secret key" / "Region"), the inputs sit crammed edge-to-edge, and the sheet reads as a broken product. Owner verdict, verbatim: "the inputs are overlapping each other, no fucking space … the text is overlapping, and overflowing each other". This is a release blocker against the standing product bar (#66).

## Root cause (traced, two cooperating bugs)

1. **The sheets reuse the CREATE form's grid.** `ConnectSheet` renders its fields inside `.form-grid` (SettingsDialog.tsx:424) — a 4-column template built for the create form's file/provider/speed/action row: `minmax(0,1fr) minmax(200px,240px) 120px auto` (theme.css:590). The settings sheet has different content: four credential fields, one of them carrying a ~90-character label.
2. **Labels are forbidden to wrap.** `.field-label { white-space: nowrap }` (theme.css:615) makes that 90-char label's min-content width ≈ the entire sheet. The `auto` column cannot shrink below it; the `1fr` column collapses to 0; tracks land on top of each other and the nowrap labels paint straight across their neighbors. The result is exactly the owner's screenshot.

The 90-character label itself ("Bucket URL for the Test button (e.g. https://s3.us-west-004.backblazeb2.com/my-bucket)") is also a copy defect — even wrapped, it is noise where the note two lines above already says the same thing.

## Design note (the fix)

- **New `.sheet-grid`** for the connect sheets: ONE field per row (`grid-template-columns: minmax(0,1fr)`), a 14px gap, every field full-width. The sheets are the #82 "one honest row" surface — stacked full-width fields are also how Google/Microsoft's own account panels render credential forms. `.form-grid` stays byte-identical for the create form.
- **Labels wrap.** `.field-label` drops `white-space: nowrap`; the label is 12px secondary text that flows. Overflow-wrap safety on long tokens.
- **Shorter copy.** The S3 endpoint label becomes `Bucket URL (the Test button's target)` — the example URL and the warning already live in the note above the field.
- No command/IPC/engine change; presentation only.

## RED first (pins written before the fix, all fail at current HEAD)

1. ConnectSheet renders its fields inside `.sheet-grid`, never `.form-grid` (component test on the real schema).
2. theme.css `.field-label` carries no `white-space: nowrap` (anatomy.test.tsx file pin — the established convention).
3. The S3 endpoint label from the vault schema is ≤ 60 chars and non-empty (schema pin).

## Acceptance

- [ ] The three RED pins exist and fail at current HEAD; the failing output is on this issue.
- [ ] Fix lands; the S3 multipart sheet renders one field per row, labels wrap, nothing overlaps at 640px and at 400px sheet widths.
- [ ] CreateJobForm's grid is untouched (its own pins still green).
- [ ] Full shell battery green (tsc, vite build, vitest) + CI 3-OS green on the landing commit.
""",
    },
    {
        "title": "The settings dialog and command palette go translucent under Windows 11 Mica — app content bleeds through the modal",
        "labels": ["bug", "ium-app", "design", "product"],
        "body": """## The defect (owner evidence, v0.2.4 on Windows 11)

Owner verdict, verbatim: "the modal pop up has transparent background, the text is overlapping, and overflowing each other". In the screenshot the dialog surface is see-through: what is BEHIND the modal (the app's own content, the wallpaper through Mica) prints through into the dialog's text area. The dialog reads as a ghost layer, not a surface.

## Root cause (traced)

#92's material program makes the window backing translucent on Windows 11:

```css
[data-material="mica"] { --surface: rgba(255, 255, 255, 0.84); }
[data-material="mica"][data-theme="dark"] { --surface: rgba(14, 21, 33, 0.82); }
```

That override hits EVERY consumer of `--surface` — including the two FLOATING surfaces, `.modal` (theme.css:935) and `.cmdk` (theme.css:977). Behind the modal sits the scrim (0.55 alpha), behind that the transparent app root, behind that the wallpaper. An 82%-alpha dialog over that stack prints the world through itself. The design comment in the same file already states the law the implementation violates: "the INK TEXT tokens stay opaque — type never sits on the wallpaper."

## Design note (the fix)

- New token `--surface-solid`: the OPAQUE surface hexes (`#ffffff` light, `#0e1521` dark) defined next to the existing tokens; the Mica blocks override `--bg`/`--surface` and NEVER `--surface-solid`.
- `.modal` and `.cmdk` consume `var(--surface-solid, var(--surface))`: the dialog and the palette are solid layers above the material — exactly Windows 11's own Content-dialog behavior (solid layer + mica backdrop). Window chrome keeps the translucency; everything a user reads in a floating surface sits on an opaque layer.
- Presentation only; no TS, command, or engine change.

## RED first (pins fail at current HEAD)

1. anatomy pin: `.modal` and `.cmdk` backgrounds resolve through `--surface-solid`.
2. anatomy pin: neither `[data-material="mica"]` block sets `--surface-solid` (the token must stay opaque under the material).

## Acceptance

- [ ] Both RED pins fail at current HEAD, then pass on the fix commit.
- [ ] Dialog and palette are fully opaque in both themes, with and without the material attribute set.
- [ ] Full shell battery green + CI 3-OS green on the landing commit.
""",
    },
    {
        "title": "Connect opens a visible cmd window and hands the browser a MANGLED authorize URL (AADSTS900144 / 'Unexpected response_type') — shell-free browser handoff + CREATE_NO_WINDOW everywhere",
        "labels": ["bug", "ium-app", "engine", "release-blocker"],
        "body": """## The defect (owner evidence, v0.2.4 on Windows)

Clicking any browser sign-in (OneDrive connect shown): a **cmd console window opens in front of the user**, then the browser lands on Microsoft's error pages:

- `AADSTS900144: The request body must contain the following parameter: 'scope'.`
- `Error connecting app … Unexpected response_type request param value.`

Nothing about this is the big-tech UX bar (#66): a shell window must NEVER appear in front of the user, and the sign-in must work.

## Root cause (traced to the byte; wire-fact reproduced 2026-11-14)

`open_in_browser` (lib.rs:494) opens URLs on Windows with `cmd /c start "" <url>`. Two facts collide:

1. The authorize URL carries NO literal space (the scope rides `%20`), and Rust std only quotes arguments containing space/tab/quote — so the URL enters the command line **unquoted**.
2. cmd.exe parses every `&` in that command line as a command separator.

The URL `authorize?client_id=<id>&redirect_uri=…&response_type=code&state=…&code_challenge=…&scope=…` is therefore executed as a LIST of commands: `start "" https://…authorize?client_id=<id>` — and the browser receives **client_id and nothing else**. No scope, no response_type, no redirect_uri, no PKCE, no state. Wire-fact (curl, live):

```
GET https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=c3a906be-…  →  200
AADSTS900144: The request body must contain the following parameter: 'scope'.
```

That is the owner's first error page, byte-for-byte. The second page ("Error connecting app / Unexpected response_type") is the MSA/consumer face of the same mangled handoff. The core's `authorize_url` is CORRECT (its contract test pins scope + response_type + PKCE) — the destruction happens between the shell and the browser.

**The console window itself** is the same class of neglect, one layer down: a GUI process spawning a console-subsystem binary on Windows gets a visible console window unless `CREATE_NO_WINDOW` (0x08000000) is set. Spawn sites that flash today:

- lib.rs:434 — `rclone authorize` (browser sign-in lanes)
- lib.rs:1060 — `cmd /c ver` (the Windows-11 material probe — a flash at EVERY boot)
- ium-core adapter_rclone.rs:400 (obscure), :554 (`serve s3` — the bridge's whole transfer!), :638 (mkdir), :832 (--version), :1057 (purge)

## Design note (the fix)

1. **`open_in_browser` on Windows = `explorer.exe <url>`.** Explorer is a GUI binary: no shell parsing (the URL travels as ONE argv, `&` inert), no console window, default browser resolved by the OS — the same handoff the OS shell itself uses. macOS/Linux branches untouched.
2. **Every remaining Windows spawn carries CREATE_NO_WINDOW**, cfg-gated (`std::os::windows::process::CommandExt` for std sites; `tokio::process::Command::creation_flags` for the tokio sites). rclone's stdio is piped/null — output capture is unaffected.
3. A pure `windows_browser_command(url)` helper (the `parse_windows_build` dead-code-on-non-Windows precedent) so the contract is pinned by a test that runs on every OS: program is `explorer.exe` (NEVER `cmd`), the URL argument is byte-verbatim.
4. A `CREATE_NO_WINDOW` constant pin: 0x0800_0000.

## RED first

- The helper + constant pins are compile-fail/assert RED at current HEAD (the helper does not exist; the windows branch routes through cmd).
- The existing OneDrive authorize contract test stays green — the fix never touches the URL.

## Acceptance

- [ ] RED output on this issue; fix lands with one logical commit.
- [ ] No `cmd` in the browser handoff; no spawn site on Windows without CREATE_NO_WINDOW.
- [ ] The full OneDrive authorize URL is what reaches the browser (handoff pinned by the helper test).
- [ ] Gates + CI 3-OS green on the landing commit.
""",
    },
]

created = []
for it in ISSUES:
    payload = {"title": it["title"], "body": it["body"], "labels": it["labels"]}
    r = api("issues", payload)
    # milestone
    mp = {"milestone": MILESTONE}
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/issues/{r['number']}",
        data=json.dumps(mp).encode(),
        method="PATCH",
        headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(req) as _:
        pass
    created.append((r["number"], r["title"]))
    print(f"#{r['number']} created: {r['title'][:70]}")

print(json.dumps(created))
