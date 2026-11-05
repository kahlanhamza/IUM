# #64 Design note — the TeraBox native lane (adapter_terabox)

Date: 2026-11-06 · Status: PROPOSED · Loop: M10 (the owner's re-sequenced first rung:
#64 native adapter → #61 bridge seam → deferred lanes). Wire facts rehearsed LIVE
2026-11-06 against the real dm.terabox.com deployment — the full protocol, byte-exact
roundtrips at 2 MiB (single-slice) AND 9 MiB (3-slice), by
`scripts/wirefacts64/probe_terabox_{api,e2e,multipart,verify}.sh`.

## 1. What the wire allows (W1–W13, every fact proven live today)

| # | Fact | Consequence for the engine |
|---|------|----------------------------|
| W1 | the API surface is **`dm.terabox.com`** (www answers errno -6 with the SAME jar; c-jp/data/up hosts 31045) | adapter's base URL; regional deployments differ |
| W3 | **jsToken is self-serve**: embedded in the logged-in `/main` HTML (`fn("<128-hex>")`, URL-encoded); without it writes → errno 4000023 "need verify" | the adapter fetches it ITSELF at probe with the same ndus cookie — no owner Network-tab step |
| W4 | `precreate` (autoinit=1, path, size, block_list=[per-slice MD5s]) → `uploadid` (session-bound, embeds client IP) | probe opens the session; block_list = one MD5 per 4 MiB slice |
| W5 | `locateupload` assigns the per-session upload host (`dm1-cdata…`, expire 600 s) | host is EPHEMERAL routing — never journaled as truth; re-derived on reattach |
| W6 | `superfile2` = multipart-form (`file` field, `partseq=i`, `uploadsign=0`) on the assigned host → **part MD5 echoed back = local MD5** | built-in per-part integrity gate: echo mismatch → Corruption |
| W7 | `create` (rtype=1, path, size, uploadid, block_list) → `fs_id`; `filemanager?opera=delete` removes (errno 0) | finalize = create; drill cleanup exists |
| W8 | **verify route = PCS `method=download`** on dm.terabox.com → 200 streamed bytes (302→CDN in the first rehearsal) | streamed sha256 vs source = the verify gate |
| W10 | **multi-slice PROVEN**: 3-entry block_list (4+4+1 MiB) → errno 0, 9 MiB roundtrip byte-exact | the lane PRESCRIBES uniform 4 MiB + exact tail (the #56 machinery) |
| W11 | **out-of-order partseq ACCEPTED** (partseq 1 before 0 → 200) | completion order drives parts directly |
| W12 | **re-PUT of a done partseq = idempotent** (200, same MD5 echo) | crash-window re-sends are safe; resume = journal truth re-uploads only unconfirmed |
| W13 | **dlink/filemetas is UNRELIABLE on multi-slice files** (no dlink field), `/api/download` errno 2 | the ONLY verify route is W8's PCS method=download — dlink is a dead end for this lane |
| W9 | 2 MiB AND 9 MiB roundtrips **BYTE-EXACT**; accounts left clean | the whole protocol maps onto the adapter contract |

## 2. Naming declarations (public surface, `missing_docs` enforced)

- Module: `ium-core/src/adapter_terabox.rs` → `pub mod adapter_terabox;`
- `pub const TERABOX_BLOCK: u64 = 4 * 1024 * 1024;` (the prescribed slice size, W10)
- `pub struct TeraboxConfig { ndus: String, app_id: String, base: String }`
  — from env `IUM_TERABOX_NDUS` (+ optional `IUM_TERABOX_APP_ID` default `250528`,
  `IUM_TERABOX_BASE` default `https://dm.terabox.com` for loopback tests). The ndus
  cookie is the ONLY credential (W1); jsToken is never config (W3 self-serve);
  nothing secret ever enters the journal or logs.
- `pub struct TeraboxAdapter { … }` — impl `TransferAdapter`, registry key
  **`terabox`**; capability = `RESUMABLE` (no DYNAMIC_CHUNKS, no TAIL_RACE — the
  grid is prescribed, parts are atomic).
- Session token (journaled, the finalize-once JSON shape the manager already
  repairs): `{"uploadid":…,"path":…,"final":fs_id|null}` — uhost is NOT in the
  token (W5: ephemeral routing, re-derived by locateupload on reattach; reattach
  runs locateupload, a READ — it never creates a session, the #12 rule).
- Dependency: `md-5 = "0.10"` (RustCrypto, the sha2 family already in-tree) — the
  wire's block_list/MD5-echo language is MD5 (W4/W6).

## 3. Design decisions

- **D1 — geometry by prescription (the #56 machinery, unchanged).** Chunk *i* ↔
  partseq *i* ↔ block_list[*i*], uniform 4 MiB + exact tail (W10); declared via
  `uniform_block()` (create-time) + `ProbeReport.uniform_block` (probe-time) +
  `max_parallel = Some(1)` (no docs on concurrent partseq; risk control exists —
  the serial lane is the honest default) + `min/max_chunk_bytes = Some(TERABOX_BLOCK)`.
- **D2 — session = the uploadid; parts idempotent (W12).** probe: size check
  (journal vs on-disk), MD5 sweep (whole-file + per-slice, streamed), jsToken
  self-serve (W3), precreate(autoinit=1), locateupload, insert session, journal
  token. upload_chunk: read [begin,end) at offset, PUT superfile2(partseq=idx),
  verify the MD5 echo against the local slice MD5 (W6) — mismatch → Corruption;
  confirm = full span (parts are atomic — no in-chunk partial exists).
- **D3 — finalize = create(rtype=1) → fs_id; verify = streamed download + sha256.**
  finalize returns Some(fs_id) through the manager's finalize-once machinery (the
  #50 crash-window repair rides free — token is the JSON shape). verify: PCS
  method=download streamed through sha256 + byte count vs source (W8; W13's dead
  ends recorded so nobody re-tries dlink). errno 2 on download → Corruption.
- **D4 — loud error mapping, only proven codes.** errno -6 → `AuthRequired`
  (ndus dead — the docs/09 §7 fragility: owner re-extracts cookies);
  4000023 → `AuthRequired` (risk control — jsToken rejected); errno 2 in
  download context → `Corruption`; HTTP 429 → `Throttled`; 5xx/connect →
  `Transient`; everything else → `Unknown` (never guessed).
- **D5 — env-gated drill, no secrets in repo.** The module's live drill (the
  feishu_drive_live_drill shape): 9 MiB / 3 slices, OUT-OF-ORDER partseq (W11),
  fresh-adapter reattach from the journaled token (the crash-window shape),
  create → fs_id, download+sha256 byte-exact, cleanup delete (W7). Skips loud
  without `IUM_TERABOX_NDUS`.
- **D6 — shell wiring.** `register_adapters` gains the env-gated terabox arm
  (the S3 pattern: a machine without `IUM_TERABOX_NDUS` simply has no `terabox`
  in the registry — the UI reflects the registry, never assumes).

## 4. RED plan (tests before implementation)

`ium-core/tests/adapter_terabox_contract.rs` pins the public surface (module,
const, config-from-env, session-token roundtrip, errno mapping, geometry) and
fails to compile at HEAD — the #52/#56 compile-fail RED pattern. After
implementation the assertions move into the module suite and the contract file
is removed.
