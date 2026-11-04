# Design note — the tus concatenation lane (ADR-011 extension)

Grounded in a **live wire rehearsal** on the pinned reference server
(tusd 2.10.1, Linux/x86_64, `scripts/issue50_wire_rehearsal.sh`), per the
M3 discipline: wire facts BEFORE Rust.

## 1. Wire facts (captured live, this issue's vectors)

| # | Probe | Result on tusd 2.10.1 |
|---|-------|------------------------|
| 1 | `OPTIONS /files/` | `Tus-Extension: creation,creation-with-upload,termination,concatenation,creation-defer-length` — **concatenation advertised** |
| 2 | `POST` + `Upload-Concat: partial` + `Upload-Length` | `201` + absolute `Location` (no Concat-echo) |
| 3 | N concurrent `PATCH`es, one per partial | `204` + `Upload-Offset` each — **true wire parallelism, independent sessions** |
| 4 | `HEAD` partial | per-session offset truth (the #10 discipline applies per partial) |
| 5a | `POST` + `Upload-Concat: final;<u1> <u2> …` **with** `Upload-Length` | `201` + final `Location` |
| 5b | same **without** `Upload-Length` | `201` — server derives; **re-POST creates a SECOND final** (partials are not consumed) |
| 6 | `HEAD` final, immediately | `Upload-Offset == sum` — **eager materialization** |
| 7 | `GET` final | `200`, byte-exact concatenation |
| 8a | final with bogus partial URL | `404 ERR_UPLOAD_NOT_FOUND` |
| 8b | `PATCH` on the final upload | `403 ERR_MODIFY_FINAL` — **the final is immutable** |
| 8c | final with an INCOMPLETE partial | `400 ERR_UPLOAD_NOT_FINISHED` — **all partials must complete first** |
| 8d | partial without `Upload-Length` | `400 ERR_INVALID_UPLOAD_LENGTH` |
| 8e | relative URLs in the final list | `201` (accepted; we still send absolute) |

## 2. Decisions (each grounded in a fact above)

1. **Discovery** (fact 1): `probe()` gains a leading `OPTIONS`.
   `Tus-Extension` containing `concatenation` → `ProbeReport.concatenation = true`
   (new field, default `false`; journaled in the probe event — old journals
   re-derive byte-identically).
2. **Session topology** (facts 2/5): the adapter holds either a CLASSIC
   session (one URL — today) or a CONCAT session (N partial URLs + optional
   final URL). The `session_token` in the probe event carries it:
   plain URL = classic; JSON `{"concat":true,"partials":[…],"final":…}` =
   concat. Dispatch on the first byte. `reattach` parses both — resume
   re-derives from the journal, never a live re-probe (ADR-002).
3. **The fleet seed channel**: new trait method
   `probe_fleet(&self, spec, seed)` (default impl = `probe(spec)` — S3 and
   one-shot adapters untouched). The manager's Probing transition calls it
   with `seed = min(MAX_WORKERS, planned chunk count at probe)` —
   journal-derived (the pool is the plan; a job with fewer chunks than
   workers needs no extra sessions). tusd advertises concat → the adapter
   creates `N = seed` partials (fact 2) and declares
   `max_parallel: Some(N)`; no advertisement → the CLASSIC path,
   `Some(1)`, byte-for-byte today. Plain `probe()` ALWAYS stays classic —
   only the manager's `probe_fleet` activates the lane, so the existing
   adapter-level wire tests keep their exact semantics.
4. **Range partition** (facts 3/4): partial *i* owns byte range
   `[i·L, min((i+1)·L, total))`, `L = ceil(total/N)`. Chunks — planned or
   resplit — route their intersection with each range; a chunk spanning a
   boundary is TWO segments with two PATCH targets, confirms summed. Each
   partial is a sequential-offset resource: per-partial `tokio::sync::Mutex`
   serializes same-partial segments (the SESSION stays sequential; the LANE
   is parallel), different partials run concurrently. The entry-HEAD +
   bounded realign dance (#10/#43) applies PER PARTIAL.
5. **Parallelism declaration** (fact 3): `Some(N)` rides the EXISTING
   ADR-011 channel — the probe event's `max_parallel` — so
   `declared_max_parallel` → `seed_ceiling` → `fleet_for` → AIMD compose
   unchanged. The prober's measured fleet (#48) and AIMD (#49) adapt the
   waterline WITHIN the declared ceiling.
6. **Finalize** (facts 5b/6/8b/8c): new trait method
   `finalize(&self, spec) -> Result<Option<String>>` (default `Ok(None)` —
   S3's finalize-if-needed stays inside its own path, ADR-010). The manager
   journals `concat_final` `{state:"intent"}` BEFORE the POST and
   `{state:"done","final_url":…}` AFTER (transitions journaled before they
   execute). Because tusd's re-POST creates a SECOND final (fact 5b), the
   crash-mid-concat retry is: a journaled `done` URL → reuse it (HEAD/GET
   only, never re-POST); `intent` without `done` → a fresh POST. The orphan
   bound: the only lossy window is a crash AFTER the server processed the
   POST but BEFORE the `done` event landed — at most ONE inert duplicate
   final per kill (the final is immutable and unlisted, partials are never
   consumed — fact 5b — so the duplicate is harmless garbage, and the intent
   event makes the window auditable). Finalize runs ONLY at
   the all-Confirmed boundary (the `Finishing` transition) — fact 8c makes
   earlier attempts a guaranteed 400.
7. **Verify** (facts 6/8b): concat verify = HEAD the FINAL URL
   (`offset == length == source_size`; eager materialization means no
   polling shape). No final URL yet → `ok: false` — verify refuses to lie,
   same spirit as the existing read-only gate.
8. **@tus/server lane** (recorded delta): no `concatenation` in
   `Tus-Extension` → `probe_fleet` falls back to classic, `Some(1)` — the
   test asserts this loudly, same pattern as `requires_get_download`.
9. **Zero-behavior-change guard**: servers without the header take the
   classic path byte-for-byte (12/12 wire suite unchanged); `max_chunk_bytes`
   and `min_chunk_bytes` declarations are untouched.

## 3. RED plan

`tests/tus_concat.rs` (env-gated, `tus-test.sh --concat`):
raw-HTTP legs pin the §1 vectors (green at HEAD — they test the SERVER);
the adapter legs (`probe_fleet`/`finalize`/concat `upload_chunk`) are the
RED — compile-fail at HEAD (no `Upload-Concat` anywhere), per the loop's
API-contract precedent (`verify_is_required`). `ci.yml`'s tus lane gains
`--concat` when the adapter goes green.

## 4. Acceptance measurement (M8)

Per-flow-capped link (throttle proxy): prober classifies `PerFlowCap`
(SK ≥ 2×S1) → real job with `IUM_LINK_PROBE=1` → declared N partials →
AIMD waterline → **aggregate > single-S1** recorded in the drill output —
the tunnel lane finally rides every bearer it can see.
