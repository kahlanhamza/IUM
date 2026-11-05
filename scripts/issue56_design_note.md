# #56 Design note — the Feishu Drive lane (adapter_feishu)

Date: 2026-11-05 · Status: PROPOSED · Loop: M9 rung 1 (wire facts W0–W10 are on the
issue body, rehearsed live 2026-11-05 against the real drive-v1 API)

## 1. What the wire allows (summary of W0–W10)

| # | Fact | Consequence for the engine |
|---|------|----------------------------|
| W0 | `GET /drive/explorer/v2/root_folder/meta` → app root token | destination resolution needs no user OAuth |
| W1 | prepare → `upload_id`, `block_size` **fixed 4 MiB**, `block_num = ceil(size/4 MiB)`; transaction valid **24 h** | the lane PRESCRIBES geometry; planner composes with the adapter cap (ADR-011) |
| W2 | part = POST **multipart/form-data**: `upload_id`, `seq` (0-based), `size` (actual), optional `checksum` (**Adler-32** decimal), binary field named **`file`**; serial, 5 QPS, no concurrency | per-chunk integrity on the wire; lane is single-worker (ADR-012 `max_parallel=1`) |
| W3 | finish `{upload_id, block_num}` → `file_token`; missing blocks = 1062010 | finish IS the completeness gate — verify-before-declare has a second wire-level guard |
| W4 | metas + folder list expose **no size** | verify = streamed download + sha256 (W10 proves byte-exactness) |
| W5 | `DELETE /drive/v1/files/{token}?type=file` | drill cleanup; not an engine path |
| W6 | 9 MiB → 3 blocks (4+4+1), tail uploads actual size | tail chunk < 4 MiB is legal on the wire |
| W7 | **out-of-order parts accepted** (seq 1 before 0 → 200) | chunk pool completion order drives parts directly — no serialization above the QPS rule |
| W8 | wrong shapes → 99992402 (JSON endpoints) / 1061002 (multipart) | loud errors; no silent fallbacks |
| W10 | download endpoint streams content; sha256 byte-exact | verify lane proven end-to-end live |

## 2. Naming declarations (public surface, `missing_docs` enforced)

- Module: `ium-core/src/adapter_feishu.rs` → `pub mod adapter_feishu;`
- `pub struct FeishuConfig { app_id: String, app_secret: String, root_token: Option<String> }`
  — creds from env `IUM_FEISHU_APP_ID` / `IUM_FEISHU_APP_SECRET` (never from the journal).
- `pub struct FeishuAdapter { http: reqwest::Client, cfg: FeishuConfig, token: TokenCache }`
- `TokenCache { value: String, expires_at: Instant }` — tenant_access_token endpoint
  `POST /auth/v3/tenant_access_token/internal` (2 h expiry; refetch at 90% life).
- Impl `TransferAdapter` (name/plan/upload_chunk/verify/... — the #1 required-verify contract).
- Journal: reuse session/chunk records; **`upload_id` is the session URI** (the analogue of
  the tus Location / S3 upload_id). Chunk confirm = part confirm (seq, Adler-32 on send).
  No new event kinds: `chunk_confirmed` + `path/session` fields carry it (ADR-002
  journal-truth: resume re-derives done-seqs from the journal, duplicates never sent).

## 3. Design decisions

- **D1 — geometry by prescription.** `plan()` returns uniform 4 MiB chunks with an
  exact tail (W1/W6): chunk *i* ↔ part `seq` *i*, 1:1. Implemented by declaring
  `max_chunk_bytes = 4 MiB` + `min_chunk_bytes = 4 MiB` to the planner composition
  (ADR-011: adapter cap wins over the global cap for this lane) — the same shape
  `adapter_compress` uses to force n=1, here to force uniformity.
- **D2 — session = upload transaction.** Flow: plan → prepare (size = plan total) →
  journal `upload_id` → workers upload chunks as parts (completion order; W7) →
  finish(block_num = plan.len()) when all confirmed → journal `file_token` → verify
  (download+sha256) → Done. Kill-9 mid-lane: journal has upload_id + confirmed seqs;
  recovery re-uploads ONLY unconfirmed seqs. **Expiry arm:** 1061021 → re-prepare →
  new upload_id journaled → all seqs re-uploaded (old confirms void — journaled as
  `feishu_upload_id_replaced`, one new journal event kind, the only schema touch).
- **D3 — verify is download+sha256.** No size/ETag exists on the wire (W4); streaming
  download hash is the honest form and doubles as the drill's byte-exactness proof.
  No staging to clean: the file IS the destination (finalize removes nothing).
- **D4 — busy vs error.** 1061045 (can retry) and 5 QPS backoff → `ErrorKind::Busy`
  (the #44 423 pattern: uncharged, un-evicted). 1061021 (expire) → the D2 expiry arm.
  1061073 (no scope) → loud config error, job fails honestly.
- **D5 — env-gated drills, no secrets in repo.** `scripts/feishu-test.sh` (the
  `s3-test.sh` pattern): skips loud without `IUM_FEISHU_APP_ID/SECRET`; with them:
  9 MiB 3-block live upload, out-of-order completion, kill-9 mid-lane resume,
  download+sha256 verify, cleanup via W5.

## 4. RED plan (tests before implementation)

1. `ium-core/tests/adapter_feishu_contract.rs` — contract tests pinning:
   uniform-4 MiB plan mapping (chunk i ↔ seq i, tail exact),
   Adler-32 checksum framing (multipart field order/names via a loopback recorder),
   finish-completeness gate (finish only at all-confirmed; 1062010 is a loud failure),
   verify = download+sha256 (mock download stream),
   max_parallel == 1 (trait surface),
   1061045 → Busy, 1061021 → replaced-session journal event.
   These FAIL FIRST: the module does not exist (compile-fail RED, the #52 pattern).
2. Paste the failing output on #56 → implement → green.
3. Live drill (env-gated) as D5.

## 5. Honest scope notes

- The 24 h window makes the lane resumable ACROSS process restarts only while the
  transaction lives; after expiry the re-prepare arm re-uploads everything — the
  journal stays truth (ADR-002), the wire charges the bytes, the ledger records it.
- Single-worker per job: this lane will not win the fleet-sum game; it exists for
  destination reach (the owner's own storage) and exercises the planner-composition
  seam (ADR-011) a second way.
- Feishu quota facts (5 QPS parts, 10k/day) cap throughput at ~20 GiB/h theoretical;
  irrelevant at drill scale, recorded for honesty.
