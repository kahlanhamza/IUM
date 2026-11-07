#!/usr/bin/env python3
"""File issue #81 — painless auth (owner ask 2026-11-08)."""
import json, os, urllib.request

TOK = open(".secrets/gh_token").read().strip()

TITLE = "Painless auth: keychain-backed secrets + hot lane registration + test-connection at save time"

BODY = """## The bar (owner, 2026-11-08)

> "how users authenticate to the supported platforms for uploads — it should not be painful, it should be so easy"

The credentials surface exists (#66/#69: masked IPC, explicit-clear semantics, schema guard, honest removal). This issue closes the three gaps that still make auth painful.

## Wire facts (audited 2026-11-08, at f665090)

1. **G1 — secrets sit in a plaintext file.** `vault.rs` persists every field
   (secrets included) as JSON under the OS config dir. The spec pins the real
   requirement: docs/02 §Secrets — "OS keychain via `keyring` crate
   (Credential Manager / Keychain / Secret Service)". No `keyring` dependency
   exists anywhere today. docs/12 agrees: credential VALUES enter only at
   runtime, never shipped/compiled.
2. **G2 — every save demands an app restart.** SettingsDialog's own copy:
   "credentials saved — they take effect the next time IUM starts". Root:
   `register_adapters` runs once at boot; `AdapterRegistry` is a plain `Vec`
   behind `&mut self`, unreachable through `Arc<JobManager>` after handoff.
3. **G3 — users save blind.** No verification exists between "Save" and the
   first upload failing with `AuthRequired`. The engine already has the
   honest error classes (`AuthRequired`, `ExpiredSession`) — the surface
   never uses them at add time.

## Design note

### D1 — keychain-backed secrets (closes G1)

`SecretStore` seam inside the vault:

- `KeyringStore` — service `"ium"`, account `"{adapter}.{field}"`, via the
  `keyring` crate (windows-native / apple-native / linux secret-service
  features).
- `FileStore` — today's JSON vault, unchanged behavior, used as fallback
  when the OS secret service is unavailable; the file gains a
  `"fallback": true` marker the UI prints honestly ("stored in the fallback
  file vault — your OS secret service is unavailable").
- Mode is decided at open time by construction, and tests pin BOTH paths
  deterministically (the mode is a parameter; CI runs file-mode, the
  mock-backed test proves the keyring path never touches the file).

### D2 — hot lane registration (closes G2)

- Core: `AdapterRegistry.adapters` moves behind an `RwLock`;
  `JobManager::replace_adapter(&self, name, adapter) -> bool` upserts by
  `TransferAdapter::name`. In-flight jobs keep the `Arc` they resolved at
  `create_job` — a running transfer is never disturbed; the NEXT job picks
  the new config. That is the honest contract and it is what the UI copy
  will say.
- Shell: one `build_adapter(id, vault+env) -> Option<Arc<dyn TransferAdapter>>`
  factory shared by boot and by a new `apply_credentials` command; save →
  apply → the lane is live immediately. rclone/mega async constructors ride
  `tauri::async_runtime` exactly like boot does today (env-wins preserved).
- The restart sentences die; the footer says the lane re-registered and
  shows the registry names.

### D3 — test-connection at save time (closes G3)

`probe_credentials(adapter, fields, endpoint?)` builds the adapter
in-memory from the DRAFT fields (never persisted), runs that family's
lightweight authenticated check, and returns `{ok, sentence, resumable}`:

- sentences follow the copy table — "The server rejected the credentials",
  never "Something happened" (brand sheet §voice).
- families whose credentials are checkable without a job endpoint get a
  real probe first (rclone bridge: `rclone about`-class call on the
  configured remote; feishu: tenant token endpoint; terabox: an authenticated
  listing call; s3: a signed probe against the job's endpoint when one is
  provided).
- families that cannot be checked without a job return exactly
  "saved — verified on first upload" honestly. **Never a fake green check.**

## Acceptance

1. Saving credentials re-registers the lane without restart; the restart
   sentence no longer exists in the codebase.
2. Secret values never land in the vault file when the keychain is active
   (test with an in-memory keyring mock); fallback mode keeps today's
   behavior and raises the honest marker.
3. `replace_adapter` swaps config for new jobs while in-flight jobs keep
   their resolved adapter (core unit test).
4. Test-connection speaks sentences, not error codes; blind-save copy gone
   for the families that support a probe.
5. Full gates: vitest + tsc + vite build locally; core fmt/clippy/test
   green; ium-app compile proof rides CI (the #61 contract).

## RED evidence

To be posted as a comment before implementation.

Refs: #69 #66 #67 #63 #61 #65 · docs/02 §Secrets · docs/09 §8 · docs/12
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": TITLE, "body": BODY}).encode(),
    headers={
        "Authorization": "Bearer " + TOK,
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
    },
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    out = json.load(r)
print("filed:", out["number"], out["state"], "|", out["title"])
