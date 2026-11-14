## #111 CLOSED GREEN — v0.2.4 is live: the native connectors release, every rung verified

**The release: tag `v0.2.4`, release run 43605338045 SUCCESS 4/4** (macOS dmg/app; Ubuntu deb/AppImage/rpm; Windows msi/nsis; publish → IUM-dist), **CI 43604814730 SUCCESS 8/8 on `2602494`**, the final record commit `7c16428` → **CI 43606778263 SUCCESS 8/8**. The tag was cut ONCE on the verdict SHA — no move, no draft flip (the v0.2.2 lesson held).

**Every rung, verified:**

1. **Scope + pre-flight** — v0.2.3..HEAD audited: #109 + #110 + discipline commits; range pickaxe-clean; auto-snapshots are script iterations.
2. **The sweep** — 0.2.4 across workspace, shell crate, tauri.conf, package.json, both locks; the updater mock now offers an honestly-newer 0.2.5; README speaks the two native clouds (highlights + transport rows with the true verify stories); CHANGELOG `[0.2.4]` cites #109/#110.
3. **Product shots** — all six re-shot through the real harness; the version **verified on the rendered UI** by a playwright probe (status bar `v0.2.4`, About `0.2.4`), not assumed from a pin.
4. **Gates** — GREEN on the committed tree `2602494` pre-push (the disk crater mid-gate was handled by the documented clean-target recipe; the rule never bent).
5. **Tag + release run** — SUCCESS 4/4, nothing published needed a second drive.
6. **The unauth battery — ALL GREEN, 22 checks** (`scripts/verify_distro_release_v024.py`): feed 200 unauth at 0.2.4; platform keys = shipped hardware (`darwin-aarch64` / `linux-x86_64` / `windows-x86_64`); manifest sigs byte-identical to the `.sig` assets ×3; **all 6 installers sha256-exact** against `SHA256SUMS.txt` (rpm, app.tar.gz, dmg, AppImage, deb, setup.exe); latest-not-draft-not-prerelease; 15 assets, zero provenance-shaped files.
7. **Distro surface synced and live-verified** (`36bec74`): README destinations highlight + Dropbox/OneDrive transport rows; USERGUIDE 0.2.4 installer examples + the one-click native sign-in story; sanitized CHANGELOG (no issue numbers); six re-shot assets. Hygiene-asserted, live-fetch verified after push. The fine-grained token reaching IUM-dist made the dead local DISTRO_TOKEN copy a non-block.
8. **STATUS row** — the 2026-11-14 release row is on the record with every id.

**What v0.2.4 ships to users:** Dropbox and OneDrive (incl. OneDrive for Business) as native destinations — sign in through your own browser with IUM's own app identity, resumable sessions that survive a restart, server-wins realign on drifted offsets, and Done only after the provider's own integrity hash (`content_hash` / `quickXorHash`) confirms the object. Both rehearsed live against the real APIs before shipping.

The queue is clear: no open engineering lanes remain (#65/#66 stand as standing rules, #78 is the owner's design review).