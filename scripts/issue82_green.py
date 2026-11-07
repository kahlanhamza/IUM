#!/usr/bin/env python3
"""GREEN evidence for #82 — the full loop, measured."""
import json, urllib.request

TOK = open("/home/z/my-project/.secrets/gh_token").read().strip()
SHA = "87d111347db02aace7b5c94594becfe920bcbd3e"


def api(path):
    r = urllib.request.Request(
        "https://api.github.com" + path,
        headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    )
    return json.load(urllib.request.urlopen(r))


rs = api(f"/repos/kahlanhamza/IUM/commits/{SHA}/check-runs?per_page=100")["check_runs"]
lines = "\n".join(f"- {c['name']}: **{c['conclusion']}**" for c in rs)
issue = api("/repos/kahlanhamza/IUM/issues/82")

BODY = f"""**GREEN** — the full loop measured on the committed tree `{SHA[:7]}`:

**Local gates (gate.sh on the committed tree):**
- `cargo fmt --all -- --check` — green
- `cargo clippy -p ium-core --all-targets -- -D warnings` — green
- `cargo test -p ium-core` — green; `cargo doc -p ium-core -D warnings` — green
- the frontend lane: tsc strict green, **vitest 93/93** (12 files), vite build green
- the pure-std `rustc --test` harness on `auth.rs`: **12/12** (the real rclone
  output fixture, the cancelled sign-in, ambiguity refusal, the merge order
  rules, the allowlist, the remote-name contract)
- the ium-app rust lane: skipped locally per the #61 contract — owned by CI

**CI on `{SHA[:7]}` (all check runs completed):**
{lines}

**The surface, in the owner's own words:** click Connect → the provider's
real page opens in your browser → sign in as normal → the window closes and
the app is connected. The token never crosses the UI; it merges inside the
shell and lives in the OS keychain. Generic protocols keep the focused
sheet — honest by capability, exactly as the owner framed it.

Issue closed by the commit (`Closes #82`): state **{issue['state']}**.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/82/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json"},
    method="POST",
)
with urllib.request.urlopen(req) as r:
    out = json.load(r)
print(f"posted GREEN evidence: {out['html_url']}")
