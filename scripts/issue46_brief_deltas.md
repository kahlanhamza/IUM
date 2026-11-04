## Owner research brief ingested → delta extraction for the research track (#46)

The owner supplied `docs/07_Fast_Upload_Engine_Research_brief.md` (commit cb9b8ff). It independently confirms the honest framing this issue was filed with ("no algorithm exceeds the sum of the summoned bearers" = the brief's "no software can push more bytes through a link than the link carries") and contributes concrete, sourced mechanisms to each track:

**Track 1 — multipath aggregation**
- The brief's §7 gives the concrete client-side mechanism this track sketched: bind workers to different uplinks (`SO_BINDTODEVICE` on Linux, interface-scoped sockets elsewhere) and spread chunks across them — "a scheduler, not a protocol change", no kernel MPTCP needed, throughput approaches the SUM of links. IUM's independent S3 parts + the #41 prober's per-path measurements are the exact substrate.
- The brief adds relay mode (§6): nearby edge ingest accepting chunks, forwarding over a fast backbone — the cause-C answer (bad route/peering), with provider-native accelerators (S3 Transfer Acceleration) as the zero-infra variant. BBR + fq + big socket buffers live on the RELAY (the client usually cannot change the OS congestion algorithm per app).

**Track 2 — FEC / transport**
- The brief's transport matrix (§4.5) adds two sourced honesty items to the research pass: (a) HTTP/3/QUIC benchmarked gains for BULK multi-stream upload can be minimal (its wins concentrate in many-short-requests + high latency) — treat as benchmark-option, not a promised win; (b) BBR beats loss-based CUBIC on high-latency/lossy/shallow-buffer paths (up to ~30% in some topologies, not universally).

**Cause-D levers (the "line really is 3 KB/s" answer) — NEW track this issue adopts**
- §8: zstd compression (entropy-gated, skip already-compressed media), FastCDC content-defined chunking + chunk-hash dedupe (send only deltas when a prior version exists at the destination), media re-encode with a quality warning, multi-link switching (Track 1), and the sneakernet honesty row (2 GB at 3 KB/s: a USB stick wins by orders of magnitude — the tool should SAY so).

**Also validated by the brief**
- Per-provider parallelism matrix (§5): S3-style multipart = best parallel target; tus = sequential per session, parallelism requires the Concatenation extension (the M8 tus-parallel path); vendor drive APIs mostly sequential — set expectations.
- Manifest/resume shape: SQLite WAL + "trust the server over the local manifest" reconcile = IUM's journal (ADR-002) + HEAD-truth realign, already built.
- Testing: per-connection caps and total caps as separate drill scenarios = exactly the #45 ladder's two legs.

The full sourced bibliography rides in the brief; the #46 research doc will verify and cite from it plus the primary RFCs/papers (RFC 8684, RFC 6330/RaptorQ, RFC 3449 asymmetry, BBR, Sprout/Verus).
