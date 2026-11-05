# #61 Design note — the MEGA lane over the #63 bridge (the bridge seam)

Date: 2026-11-06 · Status: PROPOSED · Loop: M10, second rung in the owner's
re-sequenced order (#64 native TeraBox CLOSED → #61 MEGA → deferred lanes on go).

**Scope honesty (the owner's pivot on record)**: #61 was filed as the NATIVE MEGA lane
(client-side AES + ECDH). The #63 bridge pivot made MEGA day-one through rclone; docs/09
§6 pinned the verdict — rclone's own page confirms the MEGA protocol is UNDOCUMENTED
(C++ SDK source only), so the native crypto lane stays RESEARCH-TRACK and this rung
delivers the **bridge seam**: MEGA as a first-class named lane riding the proven #63
bridge. The native lane reopens only if the owner wants it after M10.

## 1. What the wire allows (facts already on the record)

- **#63's W2–W12** (the bridge itself, rehearsed live): single PUT + multipart with real
  UploadIds/per-part ETags, anonymous loopback serve (the `--auth-key` upstream bug),
  buckets pre-exist (`rclone mkdir` before the first probe), the port-parse readiness,
  Spawn/Connect session semantics (serve death → `NoSuchUpload` → `ExpiredSession` honest).
- **#61's rehearsal** (`scripts/wirefacts61/probe_mega_bridge.sh`, LIVE with the owner's
  credentials): login via `RCLONE_CONFIG_MEGA_TYPE=mega` + `USER` + `PASS`
  (**rclone-obscured**); 1 MiB rclone roundtrip byte-exact; **the topology wire fact** —
  `serve s3` parses the first path segment as the bucket, and a bucket-less backend must
  be served from its ROOT (`mega:`) with a top-level folder as the bucket (the first
  attempt serving `mega:ium-drill` mis-parsed `/mp.bin` as a bucket); **9 MiB 2-part
  multipart through the LIVE backend byte-exact**; purge clean.

## 2. Design decisions

- **D1 — the seam is a credential-shaped constructor, zero new S3 code.**
  `RcloneBridgeConfig::mega_from_env()`: reads `IUM_MEGA_USER` + `IUM_MEGA_PASS`
  (required — `AuthRequired` when missing), optional `IUM_MEGA_2FA`
  (`RCLONE_CONFIG_MEGA_2FA`), optional `IUM_MEGA_BUCKET` (default `ium`) and
  `IUM_MEGA_RCLONE_BIN`; produces `mode: Spawn`, `remote: "mega:"` (the ROOT — the
  topology fact), the bucket, and `extra_env` = the three/four `RCLONE_CONFIG_MEGA_*`
  variables with the pass pre-obscured via `rclone obscure` (a subprocess — the pass
  crosses an argv for that subprocess's ~10 ms life, rclone's own standard practice,
  rehearsal-proven; never logged, never journaled). Everything downstream (SigV4 inner
  lane, ETag journaling, ADR-010 verify, the prober) is #63's, inherited unchanged.
- **D2 — the lane needs a NAME.** The registry is keyed by `TransferAdapter::name` and
  the generic bridge hard-codes `rclone_bridge`. `RcloneBridgeConfig` gains
  `lane_name: String` (default `rclone_bridge` in `new`/`from_env` — byte-identical
  behavior for every existing caller); the MEGA constructor sets `mega`. Two bridge
  instances can now coexist in one registry (e.g. a MEGA lane AND an operator-configured
  generic lane); the UI select reflects both (#16: the registry is the truth).
- **D3 — shell wiring.** `register_adapters` gains the env-gated MEGA arm (the S3/TeraBox
  pattern: no env → no lane, the UI reflects the registry). `RcloneBridgeAdapter::start`
  is async (serve spawn + readiness + mkdir, a bounded ~15 s worst case) and Tauri's
  setup is sync — the shell starts the env-gated lanes under
  `tauri::async_runtime::block_on` (machines without the env pay one `std::env::var`
  check; the app-e2e CI lane has no MEGA env and stays byte-identical).
- **D4 — the drill rides the REAL backend (env-gated).** `mega_live_drill` (the
  Feishu/TeraBox drill shape): spawn the bridge against the real MEGA account → probe a
  9 MiB job → two out-of-order parts (S3 parts are order-free) → a FRESH adapter in
  **Connect mode** against the SAME serve (the D1 session semantics: the UploadId
  registry lives in the serve process; a same-serve reattach is the byte-level resume
  shape; serve death stays the honest `ExpiredSession` — #63's recorded verdict,
  inherited) → finalize → GET-back + sha256 byte-exact → `rclone deletefile` + `rmdir`
  cleanup (the account left clean). Skips loud without `IUM_MEGA_USER`/`IUM_MEGA_PASS`.
- **D5 — error surfaces stay inherited.** Bad credentials = rclone's login failure at
  readiness/mkdir → loud `Unknown` with the masked stderr; 2FA-prompted accounts answer
  the same way until `IUM_MEGA_2FA` is set. No new error taxonomy.

## 3. RED plan

`ium-core/tests/adapter_mega_contract.rs` pins the surface (`MEGA_LANE`,
`mega_from_env` env shapes, the `lane_name` field defaulting `rclone_bridge`, the MEGA
constructor's remote/bucket/env shape) and fails to compile at HEAD — the
#52/#56/#64 compile-fail RED pattern. After implementation the assertions move into the
module suite and the contract file is removed.
