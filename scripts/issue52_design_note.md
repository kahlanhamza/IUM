# Issue #52 design note — FEC lane: RaptorQ parity repairs loss without round-trips (M8#5)

> Posted on the issue per the loop (step 2). Wire facts below were rehearsed LIVE on
> this sandbox (`scripts/wirefacts52/`, run 2026-11-05) before any implementation —
> the wire-facts-first discipline. Crate: `raptorq 1.8.1` (crates.io latest), pure
> Rust, no system deps, no TLS involvement (client-side symbolization only).

## 1. Scoping restated (the issue's own honesty)

Standard third-party endpoints cannot carry parity: an S3 object with parity parts
is a corrupted object; a tus stream with interleaved parity breaks the byte-offset
contract. The relay (who decodes and uploads plain to the destination) is a separate
infrastructure issue. #52 ships **(1)'s client half behind the adapter trait** — a
`fec` pseudo-adapter wrapping any resumable lane — plus the **decoder library** the
relay will call, and (2)'s repair-tool shape as the pure `rebuild` API. Verify stays
the existing gate in shape: all group sessions must be server-complete (HEADs); the
detail string states the relay honesty (destination decode is the relay's job).

## 2. Wire facts (live, `scripts/wirefacts52`)

| # | Fact | Evidence |
|---|---|---|
| W1 | `raptorq 1.8.1`; packet wire = **4 B PayloadId + symbol** (`EncodingPacket::serialize` = `payload_id.serialize()` ++ symbol bytes); OTI wire = **12 B** (`[u8;12]`, infallible deserialize) | crate source `base.rs`; rehearsal W1/W5 |
| W2 | Block-level exact-K control: `SourceBlockEncoder::new2(id, &config, data)` — `data.len() % symbol_size == 0` ASSERTED (groups must be symbol-padded); `source_packets()` = K packets with ESI `0..K`; `repair_packets(start, R)` assigns ESI `extended_source_block_symbols(K) + start + i` (RFC 6330 §5.3.4 — the `start` param is RELATIVE to the extended count; K=8 → repair ESI starts at 18) | rehearsal W2, W11 |
| W3 | K-of-(K+R) erasure decode: dropped 1 / 8 / 16 of 80 packets (seeded erasure) → byte-exact rebuilds every time | rehearsal W3 |
| W4 | Honesty boundaries: `3 << K` → `None`; **exactly K source with 0 repair → exact rebuild** (source packets alone suffice); kept 63 < K → `None` | rehearsal W4 |
| W5 | Wire framing round-trips: packet 4+1024 = 1028 B, `deserialize` infallible (panics only on <4 B input — our framing length-guards) | rehearsal W5 |
| W6 | **Deterministic**: same input → byte-identical packets across encoder constructions | rehearsal W6 |
| W7 | Overhead accounting at real geometry (25% config): measured wire ratio 1.2503 (1 MiB / 16 KiB sym), 1.2512 (256 KiB / 4 KiB), 1.2549 (64 KiB / 1 KiB) — the delta over ideal (K+R)/K is the 4 B/packet framing | rehearsal W7 |
| W8 | CPU cost is negligible: symbolize 1 MiB + R=16 = **2.3 ms**; decode 1 MiB after 16 erasures = **3.2 ms** (release) | rehearsal W8 |
| W9 | Multi-group shape: 4 independent groups with varying K (tail groups), 8 erasures each (R=16) → **4/4 rebuilt byte-exact** | rehearsal W9 |
| W10 | Padded tail group: decoder returns the padded buffer; first `true_len` bytes exact — trim rule is caller-side | rehearsal W10 |
| W11 | Pinned vectors captured (K=8, sym=1024, block id 7): source packet 0 starts `07 00 00 00 51 e0 7b 01 e6 f9 ba fc`, first repair packet starts `07 00 00 12 28 18 33 fa 92 98 e2 67` (ESI 18 = extended(8), proving W2) | rehearsal W11 |

Compile-time API facts (pinned from the 1.8.1 registry source): `Encoder::new(data,
&config)` / `with_defaults(data, mtu)`; object-level `Decoder::decode` is STREAMING
(one packet per call, `Option` on completion); the block-level
`SourceBlockDecoder::new2(id, &config, block_length)` + `decode(IntoIterator<Item =
EncodingPacket>) -> Option<Vec<u8>>` is the fec lane's decode path (`block_length` =
PADDED group length; the decoder derives K = ceil(block_length / symbol_size)).
`OTI::new` asserts `symbol_size % alignment == 0` → symbol_size = 16 KiB, alignment 8.

## 3. Naming

- Module `fec` (`ium-core/src/fec.rs`) — pure: geometry, framing, symbolize, rebuild.
- `FecConfig { symbol_size, overhead_pct, k_default, max_groups }`; `FecGeometry`
  (derived, journaled as rules); `FecAdapter` (`ium-core/src/adapter_fec.rs`) — the
  pseudo-adapter. Registry name `fec` (host constructs; `spec.adapter = "fec"`).
- "Group" (not "block"): a resplit-stable source span [g·G, min((g+1)·G, size)) —
  avoids collision with the crate's `SourceBlock` and the engine's `Chunk`.
- Framing: `fec::framed_stream(geometry, group) -> Vec<u8>` =
  `[u32 BE true_len][u16 BE symbol_size][K+R serialized packets…]`;
  `fec::parse_stream(&[u8]) -> (true_len, symbol_size, Vec<EncodingPacket>)` —
  length-guarded (W5's panic hazard).

## 4. Design (7 points)

**D1 — Group grid derives from source size ONLY (resplit-stable by construction).**
G = K_default × symbol_size, grown (`G ← ceil(ceil(size/max_groups)/S)·S`) so the
group count never exceeds `max_groups` (4096 — bounds eager session cost). Groups do
NOT depend on the chunk pool's piece grid: a mid-flight resplit or a resume re-derive
the SAME groups from `source_size` alone. This is stronger than the journal-frozen
piece grid and needs no journal access in the adapter (ADR-002: the adapter sees
spec + lease + its own tokens).

**D2 — The wrapped lane sees ORDINARY specs (the impedance-mismatch resolution).**
The inner adapter's contract is "payload = source file bytes at lease offsets" — but
the fec payload is DERIVED (framed symbols). Resolution: the fec adapter writes each
group's framed symbol stream to a **staging file** `source_path + ".ium-fec/gNNNNNN.sym"`
(deterministic from source + geometry; written at probe, before any session; reused
if present — resume/crash-safe; removed at finalize, best-effort) and drives the
inner adapter with per-group derived specs `spec_g { source_path: staging_g,
source_size: wire_len_g, endpoint: base + "/fec/gNNNNNN" }`. The wrapped lane sees
ordinary resumable uploads of ordinary files — `FecAdapter<T: TransferAdapter>` is
genuinely lane-generic (the #52 acceptance is over tus; the per-group endpoint
derivation is the documented lane contract). Disk cost: ~1.25× source, temporary —
recorded, not hidden.

**D3 — Eager per-group sessions at probe; tokens journaled in the probe event.**
`FecAdapter::probe` symbolizes ALL groups (W8: 2.3 ms/MiB — one pass), writes staging
streams, creates one inner session per group (POST creation with `Upload-Length =
wire_len_g`), and returns `ProbeReport` whose `session_token` = JSON
`{v, sym, k, r_pct, group_bytes, groups: [inner_token, …]}` — the issue's `fec_plan
{groups, K, R, overhead}` riding the journaled probe event verbatim (ADR-002; the
exact shape that makes #50's concat lane resumable). `reattach(endpoint, token)`
parses the JSON and replays every group token into the inner lane. Per-group symbol
receipts = the chunk confirmations' `adapter_token`s (the manager journals them on
chunk rows — ADR-002, zero new event kinds).

**D4 — Declarations.** `max_chunk_bytes = Some(G)` (one chunk ⊆ one group's source
span), `min_chunk_bytes = Some(G)` — on the fec lane the floor's real job is the
RESPLIT FREEZE: #27's mid-flight resplit can never produce sub-floor pieces, so the
pool's pieces (≤ G by the cap, #8 algebra; pieces below the floor are legal when the
cap binds — "cap wins") never invalidate group geometry. `max_parallel = None`
(independent group sessions — S3-parts semantics; each worker drives a different
group). `capabilities` pass through from the inner lane (honest badges).

**D5 — Confirm semantics (spec rule #1: server-acked only, span-honest).**
`upload_chunk(lease [resume_from, end))` drives every grid group intersecting the
lease span to wire-completion through the inner lane (the inner's own HEAD-realign
absorbs drifted offsets — the #10/#44 machinery, untouched). `bytes_confirmed` counts
**source bytes of wire-complete groups ∩ [resume_from, end)** — a partially delivered
group contributes ZERO (its missing packets may be source symbols; counting any bytes
before wire-completion would be client optimism). Wire bytes (the (K+R)/K overhead
included) flow through the `ProgressReporter` as telemetry for eviction. The manager's
`apply_confirm` clamp (`min(chunk_len)`) bounds any over-count honestly.

**D6 — Verify + finalize.** `verify` = AND over groups of `inner.verify(spec_g)`
(read-only HEADs, `offset == length == wire_len_g`) with the detail string stating
the relay honesty. `finalize` removes the staging directory and returns `None`
(no protocol finalization; verify keeps read-only semantics — a crash between
finalize and verify leaves staging absent and verify unaffected).

**D7 — Degradation / opt-in / zero regression.** FEC is host-constructed: no env
vars in the engine core, no JobSpec change, no manager diff. Plain lanes and every
pre-#52 journal remain byte-identical (the suite's 199 tests are the regression gate).
Drill-side knobs (`IUM_FEC_*`) live in the drill harness only. The empty-file edge
(#5 invariant) is a first-class test: one zero-length group → header-only session
(6 B), K=0, no packets, verify HEADs 6 B.

## 5. The honest loss story (what #52 does and does NOT claim)

The #32 loopback loss model is RTO-scale STALLS (a userspace relay cannot erase TCP
bytes) — under it, parity buys nothing: both lanes complete, the fec lane pays
~1.25× wire. The drill MEASURES that honestly (ladder below). What #52 PROVES is the
erasure-repair capability the relay mode needs: any K of (K+R) rebuilds byte-exact
(W3/W9, and the e2e test seeds R erasures on the RECEIVED streams). The crossover
"fec wins when loss×RTT cost > parity overhead" becomes REAL only with an erasure
channel — the relay's per-symbol receipts (#41-follow-up lane choice + the relay
infra issue). This note records that boundary now, not after the numbers.

## 6. RED plan (before wiring)

`ium-core/tests/fec.rs` — fails to compile (E0432 `fec`) on the clean tree = the RED:
- geometry: G/K/R/S derivation, tail group, max_groups growth, empty source;
- framing: stream layout byte-exact, ESI sequencing (source 0..K, repair from
  extended(K) — W2), parse length-guarding;
- symbolize/rebuild: seeded erasures 0..=R byte-exact; >R erasures → honest failure;
  exactly-K boundary (W4);
- PINNED VECTORS (W11 bytes, raptorq 1.8.1 = the RFC 6330 implementation reference —
  the crate ships no vector suite in the published package; our capture is the pin);
- FecAdapter over a recording resumable mock: eager N sessions at probe, token JSON
  shape, lease→group confirm mapping (span-honest, partial group = 0), reattach
  replay, verify AND, finalize staging cleanup;
- e2e over the mock: full job → group streams with R erasures per group →
  `fec::rebuild` → byte-exact source (the acceptance's "R symbols lost").

## 7. Drill plan (live)

`scripts/tus-test.sh --fec` → `ium-core/tests/fec_drill.rs` (env-gated, the
`netpath_drill` shape): tusd 2.10.1 + `throttle_proxy.py --loss PCT`. Ladder
PCT ∈ {0, 1, 3, 5} × {plain tus lane, fec lane}, same file, same bytes: wire bytes
(proxy counters), wall time, stall seconds (the proxy's `LOSS stalls= extra_s=`
lines), completion + client-side `rebuild` byte-exact on the fec lane. Expected
honest result: fec wire ≈ 1.25× at every PCT (parity overhead paid); plain wire 1.0×
+ retransmit stalls growing with PCT; completion both — the §5 boundary on record.

## 8. What #52 does NOT do (future work, each its own issue)

The relay service (erasure-channel peer, per-symbol receipts, destination decode +
plain multipart re-upload); prober-driven lane CHOICE from loss/RTT (#41 follow-up);
fec × multi-path composition (#51's bonds under the fec adapter — orthogonal layers,
composition later); lazy session creation for >4096-group files beyond the G-growth
cap.
