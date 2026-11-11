#!/usr/bin/env python3
"""File the Tier-3 native-connector evaluation spike issue (post-v0.2.3 lane).

The destination lane shipped in v0.2.3: Tier 1 (presets + guided keys),
Tier 1.5 (S3 Test probe), Tier 2 (expiring share links), and the Tier-3
interim (the bridge authorize family: pCloud/Dropbox/OneDrive/Box/Drive +
native Drive #59 / YouTube #85). The pasted plan's own gate — "only after
Tier 1 proves the funnel" — has passed. This spike is the prescribed
per-provider maintenance-liability evaluation. NO code claimed by this issue.
"""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()

TITLE = ("Tier 3 — native mainstream connectors (Dropbox/OneDrive): the per-provider "
         "maintenance-liability spike — go/no-go, NO code")

BODY = """## Problem / user story

The ratified destination plan (shipped in v0.2.3, #103/#104/#107) names Tier 3 as
"true mainstream connectors — native OAuth — the real regular-user answer eventually,
but each is a per-provider maintenance liability. Only after Tier 1 proves the funnel."

Tier 1 **proved the funnel**: provider presets + guided keys + the S3 Test probe +
expiring share links shipped in v0.2.3 (release run 43062849275, battery 24/24), and the
Tier-3 *interim* is live — the connect surface (#82) gives pCloud, Dropbox, OneDrive, Box
and Google Drive real browser sign-in through the bridge `authorize` flow
(allowlist-enforced, the injection contract), with Drive native (#84→#59) and YouTube
native (#85).

So the only question left on this plan: **do we build NATIVE connectors (our own OAuth
apps) for Dropbox and OneDrive, and what does each cost to maintain?** This spike answers
it with a go/no-go per provider, from the providers' own official documentation only —
no random-Googling research, per the owner's release-bar rule (#105's discipline).

## What the spike evaluates (per provider: Dropbox, OneDrive/Microsoft Graph)

1. **App registration reality** — what an IUM-owned OAuth app requires: developer
   account tier, app creation, scopes requested, production status (development vs
   published), and whether provider-side app review/verification is triggered by our
   scope set or by distribution volume.
2. **Token mechanics** — access-token TTL, refresh-token lifetime and rotation rules,
   and what an expired/revoked refresh token must look like in our UX (the #81
   SecretStore seam holds the refresh token; the adapter contract needs the
   re-auth surface).
3. **Upload protocol fit** — the resumable-upload session endpoint and whether it maps
   onto our adapter contract (`probe`/`upload_chunk`/`finalize`/`verify`/`reattach`)
   without betraying verify-before-declare (ADR-004/ADR-010 shape), including what
   server-side integrity check exists (content hash header, quota/exists read-back).
4. **Maintenance liability ledger** — what breaks when the provider changes: versioned
   API? deprecation policy? breaking-change notice period? Historical cadence of
   breaking changes to their upload/auth surface.
5. **The bridge comparison** — what the rclone authorize flow already delivers today
   (rclone's own registered app, maintained upstream by the rclone project), and the
   honest deltas native would buy (our branding, scope minimization, no third-party
   app dependency, rate-limit identity) vs what it costs (we own every refresh/regeneration
   edge forever).

## Acceptance criteria

- [ ] A per-provider verdict posted on this issue: **GO / NO-GO / WAIT**, each with the
      five evaluation rows above filled from official docs (URLs cited, dated).
- [ ] The verdict names the smallest honest scope set per provider (read/write files
      only — no broad scopes) and the exact UX contract for token loss.
- [ ] A GO carries a filed follow-up issue per provider (the adapter lane, design-note-first,
      credential-blocked on the owner creating the app registration).
- [ ] A NO-GO records what the bridge path must keep guaranteeing instead (and files
      nothing).
- [ ] **No production code in this spike.** The first code change rides the follow-up
      issue's own RED cycle.

## Out of scope

- Google Drive / YouTube — already native (#59/#85).
- pCloud / Box — bridge covers them; native only if a user-facing failure is on record.
- Social publishing (TikTok/IG/FB) — decided separately by the #62 spike verdict.
- Any relay-service design — the keys-stay-local promise is a README selling point;
  a credential-holding relay is banned by the plan's own "what we should never do" list.

## Boundary register (docs/08 §4 — mandatory since #31)

Research-only spike: zero entities touched, zero edges at risk. The follow-up GO lane
(if any) files its own boundary rows — the adapter seam (`probe`/`verify`/`reattach`,
ADR-002 journal-truth, ADR-011 declaration channel) is the surface it will touch, and
`tests/verify_is_required.rs` + the trybuild contract remain the guards.

## Refs

The owner's tiered destination plan (ratified in-session, shipped through #103/#104/#107);
#82 connect surface; #81 SecretStore/keyring seam; #59 Drive native; #85 YouTube native;
#62 the socials-spike precedent (research spike, go/no-go, NO code — the shape this issue
follows); docs/05 "Real data only"; the owner's release-bar research rule (#105).
"""

MILESTONE_TITLE = "M11 — The shipping bar: installers, zero-touch, the funnel after v0.2.3"
LABELS = ["enhancement", "process"]


def api(method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"https://api.github.com/repos/kahlanhamza/IUM/{path}",
        data=json.dumps(payload).encode() if payload else None,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def main() -> int:
    # milestone lookup (M11 = number 10 on GitHub)
    ms = api("GET", "milestones?state=all&per_page=50")
    mid = next((m["number"] for m in ms if m["title"].startswith("M11")), None)

    issue = api(
        "POST",
        "issues",
        {"title": TITLE, "body": BODY, "labels": LABELS,
         **({"milestone": mid} if mid else {})},
    )
    print(f"filed: #{issue['number']} {issue['title'][:70]} (milestone={mid})")
    return 0


if __name__ == "__main__":
    sys_exit = main()
