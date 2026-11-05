# Issue #53 design note — Cause-D byte reduction: entropy-gated zstd transfer mode (M8#6)

> Posted on the issue per the loop (step 2). Wire facts rehearsed LIVE on this
> sandbox (`scripts/wirefacts53/`, run 2026-11-05) before any implementation.
> Crate: `zstd 0.13.4` (zstd-sys 2.1.2 + zstd 1.5.7, pure C build via cc — no
> system dep).

## 1. Wire facts (live)

| # | Fact | Evidence |
|---|---|---|
| W1 | Round-trip with `include_checksum(true)` is byte-exact; 1 MiB text @ level 19 → ratio ≈ 0.06 | rehearsal W1 |
| W2 | The gate DISCRIMINATES at the 1.15 bar, fast level 3, head/mid/tail 64 KiB samples: text 6.051 → ACCEPT; video-like 1.000 → DECLINE | rehearsal W2 |
| W3 | Level 19 costs ~1.26 s per 4 MiB of text — vs 87 s of wire PER KIB at the owner's 3 KB/s regime; compression is free against that wire | rehearsal W3 |
| W4 | A corrupted archive FAILS decode loudly ("Data corruption detected") — the checksum integrity gate is real | rehearsal W4 |
| W5 | `statvfs` works unprivileged (uid 1001): the #34 disk-space check is implementable in-process | rehearsal W5 |
| W6 | File-to-file streaming encode via `io::copy` is the adapter's shape (bounded memory) | rehearsal W6 |

## 2. Naming

Module `compress` (`ium-core/src/compress.rs` — the entropy gate + manifest,
pure-ish); adapter `adapter_compress.rs` → `CompressAdapter` ("compress"),
wrapping any resumable lane. `GateDecision { Accept { ratio } | Decline { ratio,
why } }`.

## 3. Design (6 points)

**D1 — The entropy gate is a pure function.** Samples: head / middle / tail
64 KiB of the REAL bytes (files < 192 KiB sample whole); each sample compressed
at fast level 3; `projected_ratio = Σraw / Σpacked`; `< 1.15` → Decline (the
honest "a USB stick wins" line — journaled with the ratio, never pretended).

**D2 — Transfer mode = the single-chunk shape (the impedance resolution).**
The manager plans chunks over the SOURCE span, but the archive is smaller —
piece-grid mapping would be fiction. Resolution: `CompressAdapter` declares
`min_chunk_bytes = Some(source_size)` → `clamp_by_chunk_min` forces n = 1 (one
piece spanning the whole source; resplit is floor-blocked; `replan_measured`
keeps n = 1). `upload_chunk` drives the WHOLE archive through the inner lane as
ONE resumable upload (spec_z: source_path = `<source>.zst`, source_size =
archive_size): **every drill property (kill-9, zero re-upload, 409-realign,
verify) is inherited by the archive bytes unchanged** — the issue's point 3,
structurally. `max_parallel = Some(1)` (one archive session, sequential).

**D3 — Progress is a journaled linear projection.** Confirm = inner-acked
archive delta × (source_size / archive_size), clamped to the chunk's span — the
final delta absorbs rounding. The projection is explicit in the manifest
(mode/level/both sizes/ratio): the UI's percent stays in SOURCE space, the wire
truth stays in `ProgressReporter` telemetry. The REAL integrity gate is verify
(D4), not the projection.

**D4 — Verify = three gates in one pass.** (a) inner's byte-exact HEAD on the
ARCHIVE; (b) local zstd decode of the archive — checksum-validated (W4); (c)
sha256 over the DECODED bytes == the journaled source hash, byte count ==
source_size. The VerifyReport detail states all three. Destination semantics
honest: the manifest (probe-event token JSON) carries mode, level,
source_sha256, archive_sha256, archive_size, ratio — the stored-object naming
(`<name>.zst`) is the host's mapping (tus ids are server-generated; W13);
the manifest is what the UI's "stored compressed, N% saved" reads.

**D5 — Decline paths are plain jobs.** Gate decline, `IUM_NO_COMPRESS=1` (the
kill-switch, mirroring #48's instrument channel), and insufficient disk
(statvfs, #34 discipline: free < source×1.1) all route to: inner.probe with the
ORIGINAL spec, token JSON `{mode:"declined", why, ratio}` journaled, zero
behavior change otherwise.

**D6 — Cleanup + level.** Done → `finalize` removes the staging `.zst`
(verified ⇒ no longer needed). **Failed/Cancel KEEP it deliberately** — a
deviation from the issue's literal ask, on the record: cancel-then-resume is a
supported flow (#15) and the archive IS the transfer's resumable state; a fresh
job with the same source reuses it (deterministic re-derive if absent). Level =
min(19, 3 + 3 × available_parallelism), journaled (the issue's "CPU cores"
auto; capped 19).

## 4. RED plan

`tests/compress.rs` (RED = E0432 on the clean tree): gate pure-fn
(accept/decline + ratio honesty + mid-only compressibility), adapter-over-mock
(accepted → inner sees spec_z, manifest JSON shape; declined → inner sees the
original spec; IUM_NO_COMPRESS; projection sums to source_size), verify-over-real-archive
(decode+hash == source hash). Live drill `tests/compress_drill.rs`
(`tus-test.sh --compress`): compressible 4 MiB fixture through the loopback tus
lane — wire < source, both hashes journaled, archive read back + decode + hash
== source, temp cleaned; decline leg leaves zero temp files.

## 5. What #53 does NOT do

Delta/dedupe against a previous destination version (the brief's separate
lever); parallel/many-chunk compressed transfer (the mode targets the
wire-bound regime); UI surfaces (Sprint-2 reads the manifest).
