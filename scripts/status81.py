#!/usr/bin/env python3
"""STATUS.md update for the #81 loop (+ the misplaced session-row fix)."""
import re

p = "STATUS.md"
s = open(p).read()

# 1. Top "Last updated" block — prepend the M12#2 paragraph (newest first).
anchor = "> Last updated: **(M12#1 LANDED (#80):"
m12_top = (
    "> Last updated: **(M12#2 LANDED (#81): painless auth — the owner's bar \"it should not be "
    "painful, it should be so easy\" turned into three measured gaps, closed in one loop. THE "
    "WIRE FACTS that scoped the lane: the #69 vault existed (masked IPC, explicit-clear "
    "semantics, schema guard) but (G1) secret VALUES sat as plaintext JSON in the OS config "
    "dir — spec §Secrets pins the OS keychain via the keyring crate and no keyring existed "
    "anywhere; (G2) every save demanded an app restart (register_adapters ran once at boot; "
    "AdapterRegistry was a plain Vec behind &mut self, unreachable through Arc<JobManager>); "
    "(G3) users saved blind (no verification between Save and the first upload's "
    "AuthRequired). (1) KEYCHAIN (G1): the SecretStore seam — vault.rs stays pure-std (the "
    "standalone rustc --test harness keeps working), the real keyring impl rides lib.rs (CI "
    "compile proof per the #61 contract); secret values go to the OS keychain (service "
    "\"ium\", one entry per {adapter}.{field}); no secret service (headless Linux/CI) = the "
    "honest 0600-file fallback with the secret_mode marker the Settings footer prints; "
    "legacy plaintext migrates at the first save (old values keep answering until then — "
    "nothing breaks, then the file is scrubbed); the masked-IPC contract untouched. (2) HOT "
    "LANES (G2): the registry moved behind an RwLock — JobManager::replace_adapter/"
    "remove_adapter upsert through the SHARED Arc (in-flight jobs keep the Arc they resolved "
    "at create_job; the swap is never retroactive — core contract tests in "
    "tests/hot_registry.rs); a first credential entry registers a lane boot never had; "
    "removing credentials removes the lane; apply_credentials rebuilds the lane from "
    "vault+env through the shared build_adapter factory (boot + hot apply = ONE code path) — "
    "save/remove re-registers LIVE, the three restart sentences are dead from the codebase. "
    "(3) TEST-CONNECTION (G3): check_credentials on terabox (jsToken gate + one /api/quota "
    "read; errno -6 = the honest re-extract sentence) and feishu (tenant token + root-folder "
    "meta; bad app creds fail at the token) — metadata reads ONLY, no upload session a Test "
    "button could orphan; the bridge families prove serve spawn + readiness and the sentence "
    "says exactly what that proves; the s3 family answers \"Not checked here — this lane "
    "verifies itself on the first upload\" — never a fake green check. The probe builds "
    "configs from DRAFTS in memory (never persisted, never env; rclone_obscure made pub for "
    "the mega drafts); failures route through the #70 copy table with the raw preserved. "
    "RED: E0599 (no replace_adapter through the Arc) + ERR_MODULE_NOT_FOUND ./auth at HEAD "
    "f665090 (comment 6711213801). En route the standalone vault harness caught 2 real "
    "defects pre-commit (a format! typo; the migration test's own tmpdir wiped its seed — "
    "test bug, not product). Gates: vitest 84/84 (75+9), tsc strict, vite build 197.98 KB, "
    "fmt clean, clippy -D warnings, core suite 284 green (cargo exit 0; +6 check_credentials "
    "+ 3 hot_registry), vault harness 13/13 (9+4), gate.sh on the committed tree; shell Rust "
    "compile proof = CI) "
) + "(M12#1 LANDED (#80):"
s = s.replace(anchor, m12_top, 1)

# 2. Done table — new row right under the header.
done_anchor = """| Date | Item | Evidence |
|---|---|---|
| 2026-11-01 | Sprint 0:"""
done_row = """| Date | Item | Evidence |
|---|---|---|
| 2026-11-08 | **Issue #81 closed**: painless auth — (1) keychain-backed secrets (SecretStore seam; the keyring crate impl rides lib.rs; the honest 0600-file fallback marked `secret_mode` and printed in Settings; legacy plaintext migrates at the first save); (2) hot lane registration (RwLock registry; `replace_adapter`/`remove_adapter` through the shared `Arc<JobManager>`; the shared `build_adapter` factory; `apply_credentials` — no restart, ever); (3) Test connection (`check_credentials` on terabox/feishu — read-only metadata reads, no upload session; bridge serve-spawn proof; the s3 family answers "verified on the first upload"); the three restart sentences are dead; the probe's failures speak the #70 copy table, raw preserved | RED comment 6711213801; vitest 84/84; core 284; vault harness 13/13; CI pending |
| 2026-11-01 | Sprint 0:"""
s = s.replace(done_anchor, done_row, 1)

# 3. Backlog 32.c — mark done.
c_anchor = """    c. **Accounts / OAuth layer** — token storage via OS keyring (tauri plugin), account
       model in the shell, adapter auth injection — design note FIRST (touches the
       adapter trait's endpoint/token seam)."""
c_done = """    c. ~~**Accounts / OAuth layer** — token storage via OS keyring (tauri plugin), account
       model in the shell, adapter auth injection — design note FIRST (touches the
       adapter trait's endpoint/token seam).~~ ✅ (2026-11-08, #81: the SecretStore seam
       + the real keyring crate behind it; the masked credential forms ARE the account
       model for the server-style lanes; auth injection = the shared `build_adapter`
       factory + `apply_credentials` (hot, no restart); the OAuth families stay parked
       with #59/#60 — the bridge's `authorize` flow is the designed path when the owner
       unparks them.)"""
s = s.replace(c_anchor, c_done, 1)

# 4. Backlog 32.d — the auto-update decision is done (#80).
d_anchor = """    d. **Release engineering toward v0.1.0** — tauri bundle signed installers (3 OS),
       auto-update channel decision, CHANGELOG 0.1.0 cut per the semver-0.x policy."""
d_done = """    d. **Release engineering toward v0.1.0** — tauri bundle signed installers (3 OS),
       ~~auto-update channel decision~~ ✅ (2026-11-08, #80: the updater runtime + the
       latest.json pipeline shipped — the channel is `releases/latest/download/latest.json`,
       check automatic, install user-armed), CHANGELOG 0.1.0 cut per the semver-0.x policy."""
s = s.replace(d_anchor, d_done, 1)

# 5. Restore the two misplaced session-log rows from the defect-audit table.
row81 = None
row79 = None
m = re.search(r"\n\| 2026-11-08 \(M12#1 — auto-update[^\n]*\n\| 2026-11-07 \(M11#11[^\n]*\n", s)
if m:
    block = m.group(0)
    row81 = "| 2026-11-08 (M12#1 — auto-update" + block.split("| 2026-11-08 (M12#1 — auto-update", 1)[1].split("\n| 2026-11-07 (M11#11", 1)[0].rstrip("\n")
    row79 = "| 2026-11-07 (M11#11" + block.split("\n| 2026-11-07 (M11#11", 1)[1].rstrip("\n")
    s = s.replace(block, "\n", 1)

# 6. Session log — new M12#2 row at the top, then the restored rows.
log_anchor = """| Date | Task | Outcome | Commit |
|---|---|---|---|
| 2026-11-07 (M11#10"""
log_head = """| Date | Task | Outcome | Commit |
|---|---|---|---|
""" + (row81 + "\n" if row81 else "") + (row79 + "\n" if row79 else "")
new_row = (
    "| 2026-11-08 (M12#2 — painless auth: keychain, hot lanes, Test connection, #81) "
    "| **Full loop on the owner's product question** (\"how users authenticate to the "
    "supported platforms — it should not be painful, it should be so easy\"). Wire facts: "
    "plaintext secret file (spec §Secrets pins the keyring), restart-required saves, blind "
    "saves — three gaps, one issue with the design note. RED (E0599 through the Arc + "
    "./auth missing, comment 6711213801); the swap: SecretStore seam + KeyringStore "
    "(keyring crate, per-OS features), RwLock registry + replace_adapter/remove_adapter + "
    "the build_adapter factory shared by boot and apply_credentials, check_credentials on "
    "terabox/feishu (read-only metadata reads, no upload session), probe_credentials + "
    "secret_mode commands, the Settings Test control + the live-apply copy (the three "
    "restart sentences dead), legacy plaintext migrates at the first save. En route the "
    "standalone vault harness caught 2 real defects pre-commit (a format! typo; the "
    "migration test's own tmpdir wiped its seed — test bug, not product). Also restored: "
    "the M12#1 and M11#11 session rows sat inside the defect-audit table — moved here "
    "| vitest 84/84; tsc; build 197.98 KB; fmt clean; clippy -D warnings; core 284 green "
    "(+6 check_credentials, +3 hot_registry); vault harness 13/13; shell Rust compile = CI "
    "| this commit |\n"
) + "| 2026-11-07 (M11#10"
s = s.replace(log_anchor, log_head + new_row, 1)

open(p, "w").write(s)
print("STATUS updated;",
      "row81 restored" if row81 else "ROW81 NOT FOUND",
      "| row79 restored" if row79 else "| ROW79 NOT FOUND")
