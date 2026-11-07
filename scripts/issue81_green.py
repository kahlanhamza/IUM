#!/usr/bin/env python3
"""Post the #81 GREEN evidence comment."""
import json, urllib.request

TOK = open(".secrets/gh_token").read().strip()

BODY = """## GREEN — landed at `5debf2a`, CI verdict GREEN at `c8f2213` (run 42404650773-class, all 8 checks)

**The three gaps, closed:**

1. **Keychain-backed secrets (G1)** — the `SecretStore` seam carries secret VALUES to the OS keychain (service `ium`, one entry per `{adapter}.{field}`; Windows Credential Manager / macOS Keychain / Linux Secret Service via the `keyring` crate, per-OS features). No secret service available (headless Linux, CI) = the honest 0600-file fallback, marked `secret_mode` in the JSON and printed in the Settings footer. Legacy plaintext migrates at the first save (old values keep answering until then; the file is scrubbed after). `vault.rs` stays pure-std — the standalone `rustc --test` harness caught 2 real defects pre-commit. Harness **13/13** (9+4: keychain-never-in-file, file-fallback marker, legacy migration, delete-clears-keychain).
2. **Hot lane registration (G2)** — the registry behind an `RwLock`; `JobManager::replace_adapter`/`remove_adapter` upsert through the SHARED `Arc<JobManager>` (in-flight jobs keep the Arc they resolved at `create_job`; the swap is never retroactive — contract tests in `ium-core/tests/hot_registry.rs`). `apply_credentials` rebuilds the lane from vault+env through the shared `build_adapter` factory (boot + apply = one code path). The three "the next time IUM starts" sentences are dead from the codebase; save/remove re-registers live.
3. **Test connection (G3)** — `check_credentials` on terabox (jsToken gate + one `/api/quota` read; errno −6 = the honest re-extract sentence) and feishu (tenant token + root-folder meta; bad app creds fail at the token): **metadata reads only, no upload session a Test button could orphan** (loopback wire tests pin both verdict paths ×3). The bridge families prove serve spawn + readiness and the sentence says exactly what that proves; the s3 family answers "Not checked here — this lane verifies itself on the first upload" — never a fake green check. Probes build configs from DRAFTS in memory (never persisted, never env; `rclone_obscure` now pub). Failures route through the #70 copy table with the raw preserved ("The destination rejected the credentials — … the engine said: …").

**Gates:** vitest **84/84** (75+9 — the probe state machine + the live-apply copy + the restart-sentence ban) · tsc strict · vite build 197.98 KB · fmt clean · clippy `-D warnings` · core suite **284 green** (cargo exit 0; +6 `check_credentials` + 3 `hot_registry`) · vault harness 13/13 · gate.sh GREEN on the committed tree `5debf2a` · the shell Rust compile proof rides CI (the #61 contract — and it earned its keep: CI caught the one error the sandbox structurally cannot, `rclone_obscure` lives on `RcloneBridgeConfig` not the Adapter, fixed in `c8f2213`, re-run **ALL GREEN**, all three OSes incl. app-e2e).

**What "auth" now means for a user:** enter the credentials once in Settings (masked, keyed by the OS), press Test connection and read an honest sentence before any upload moves, press Save and the lane is live immediately — no restart, no blind first upload, and the OS keeps the secret, not a JSON file.

Closes the loop opened by the owner's question; the WebDAV transport consumes this seam next.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/81/comments",
    data=json.dumps({"body": BODY}).encode(),
    headers={"Authorization": "Bearer " + TOK, "Accept": "application/vnd.github+json", "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    out = json.load(r)
print("green comment:", out["id"])
