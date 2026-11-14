#!/usr/bin/env python3
"""Post the RED evidence comments on #113/#114/#115 (loop step 3)."""
import json
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def comment(issue: int, body: str) -> None:
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/issues/{issue}/comments",
        data=json.dumps({"body": body}).encode(),
        headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(req) as r:
        out = json.load(r)
        print(f"#{issue} comment {out['id']}")


comment(113, """**RED** — measured at current HEAD before any fix. The owner's screenshot IS these numbers.

1. New pins `ium-app/src/settings.sheet.test.tsx` (real SettingsDialog render, real theme.css read) — **7 failed / 1 passed**:

```
× the connect sheets' layout contract (#113) > field labels WRAP — nowrap is what painted the S3 label across its neighbors
× the connect sheets' layout contract (#113) > the sheet grid exists: ONE field per row, full width, breathing room
× the connect sheets' layout contract (#113) > the create form's 4-column grid stays for the create form only
× the connect sheets' layout contract (#113) > the S3 sheet renders its fields inside .sheet-grid — one per row
  (actual: the sheet still renders its 4 fields inside the create form's .form-grid)
```

2. The vault-schema pin (the 90-char label, measured):

```
$ cargo test (vault harness, src-tauri/src/vault.rs)
test tests::the_s3_endpoint_label_is_one_line_of_copy ... FAILED
  "label is sheet noise (88 chars): Bucket URL for the Test button (e.g. https://s3.us-west-004.backblazeb2.com/my-bucket)"
test result: FAILED. 17 passed; 1 failed
```

Baseline: core 53 suites / 376 passed / 0 failed; the 8-pin file is the only red on the tree. Implementation follows.
""")

comment(114, """**RED** — measured at current HEAD (same pins file as #113, the floating-surface half):

```
× floating surfaces stay opaque under every material (#114) > the settings modal consumes --surface-solid, not the material-translucent --surface
  (actual: block(".modal") carries `background: var(--surface)` — the token #92's mica blocks override to rgba(…,0.82-0.84))
× floating surfaces stay opaque under every material (#114) > the command palette consumes --surface-solid too
  (actual: block(".cmdk") carries `background: var(--surface)`)
× floating surfaces stay opaque under every material (#114) > --surface-solid exists and is opaque in both themes
  (actual: the token does not exist in theme.css)
```

The one green pin in the file is the regression guard (the mica blocks must never set `--surface-solid`) — it holds before and after by construction. Implementation follows.
""")

comment(115, """**RED + wire-facts** — the mangled handoff reproduced live against Entra before any code moved.

**Wire-fact A (the owner's first error page, byte-for-byte).** The authorize URL carries no literal space (the scope rides `%20`), so Rust std does not quote it; `cmd /c start "" <url>` lets cmd.exe execute every `&` as a command separator. The browser therefore receives `authorize?client_id=<id>` and NOTHING else:

```
$ curl "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=c3a906be-d85b-4409-b3cd-bc152ae58550"
→ 200 … AADSTS900144: The request body must contain the following parameter: 'scope'.
```

That is the owner's screenshot verbatim. The second page ("Error connecting app / Unexpected response_type request param value") is the same mangled handoff's MSA/consumer face.

**Wire-fact B (core is innocent).** `ium-core::onedrive::authorize_url` is contract-tested (scope + response_type + PKCE + exact redirect pinned) — the destruction happens between the shell and the browser.

**RED (the rust harness, `rustc --test auth.rs` — pure-std, zero deps):** the two contract tests exist at HEAD, the contracts do not:

```
$ rustc --test auth.rs -o /tmp/auth_red
error[E0425]: cannot find value `CREATE_NO_WINDOW` in this scope
error[E0425]: cannot find function `windows_browser_command` in this scope
```

Baseline: core 53 suites / 376 passed / 0 failed. The spawn inventory audited for the same class (console flash): lib.rs:434 (`rclone authorize`), lib.rs:1060 (`cmd /c ver` — a flash at EVERY boot), ium-core adapter_rclone.rs:400/554/638/832/1057 (obscure / `serve s3` / mkdir / --version / purge). Implementation follows.
""")
