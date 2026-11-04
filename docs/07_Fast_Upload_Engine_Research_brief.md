# Fast Upload Engine: Architecture and Technical Design

*An "IDM for uploads" design, targeting a 2 GB file in minutes instead of days.*

## 1. Read this first: what is physically possible

A 2 GB file needs a minimum sustained rate to finish in a given time:

| Target time | Required rate | In megabits |
| --- | --- | --- |
| 5 minutes | \~6.8 MB/s | \~55 Mbps |
| 10 minutes | \~3.4 MB/s | \~27 Mbps |
| 30 minutes | \~1.1 MB/s | \~9 Mbps |
| 8 days (your current case) | 3 KB/s | 0.024 Mbps |

No software can push more bytes through a link than the link carries. So the whole design hinges on **why** the speed is 3 KB/s. There are only four possible causes, and each has a different fix:

| Cause | Symptom | What the tool can do |
| --- | --- | --- |
| **A. Per-connection throttling** by the server or CDN | Speed test shows a fast line, but one upload crawls | Parallel connections (big win) |
| **B. Single-stream TCP limits** (high latency, small window, loss-based congestion control) | Fast line, long distance, one stream far below line speed | Parallel streams, BBR, QUIC, nearby edge ingest (big win) |
| **C. Bad route / peering to the destination** | Speed to other sites is fine | Relay through a nearby edge or another path (big win) |
| **D. The access line itself is 3 KB/s** | Speed test also shows \~3 KB/s upload | Shrink the data, add a second link, or move the data physically (limited) |

**Design principle:** the engine's first job is to *diagnose which cause applies* and then pick the strategy. Causes A to C can give 10x to 1000x speedups. Cause D cannot be beaten by software, only reduced by sending fewer bytes or adding links.

## 2. Goals and non-goals

**Goals**

- Saturate whatever the real available upstream bandwidth is, with minimal tuning by the user.
- Survive disconnects, sleep, and crashes without losing progress.
- Verify integrity end to end.
- Work with multiple destinations through one engine.

**Non-goals**

- Exceeding the physical line rate.
- Parallelizing uploads to generic web forms that accept one plain POST (no server cooperation, no chunking possible).

## 3. High-level architecture

```
┌─────────────────────────── Client app (CLI / desktop / browser extension) ───────────────────────────┐
│  UI layer  ──►  Core engine (Rust or Go)                                                              │
│                 ├─ Probe & Diagnose module                                                            │
│                 ├─ Chunk planner                                                                      │
│                 ├─ Pipeline: Reader ► Hasher ► (Compressor) ► Uploader workers                        │
│                 ├─ Adaptive concurrency controller                                                    │
│                 ├─ Connection manager (pools, HTTP/1.1, HTTP/2, HTTP/3)                              │
│                 ├─ Manifest store (SQLite)                                                            │
│                 ├─ Retry / backoff engine                                                             │
│                 └─ Provider plugins: S3 · tus · GDrive · OneDrive · Dropbox · Custom                  │
└───────────────────────────────────────────────┬───────────────────────────────────────────────────┘
                                                │ parallel streams (optionally multiple NICs)
                    ┌───────────────────────────┴──────────────────────────┐
                    │ Direct to destination      OR     Optional edge relay │
                    └───────────────────────────┬──────────────────────────┘
                                                ▼
                        Object storage / tus server / cloud drive API
```

The two deployment modes:

1. **Direct mode (no server of your own):** the client talks straight to the destination's multipart or resumable API. Easiest, covers causes A and B.
2. **Relay mode (optional edge ingest service):** the client sends to a nearby relay you run (or a provider's accelerated endpoint), and the relay forwards over a good backbone. Covers cause C and some of B.

## 4. Core components

### 4.1 Probe and diagnose

Run before the first byte of real data, takes about 10 to 20 seconds:

1. **Line test:** upload a small test payload (e.g. 8 MiB) to a probe endpoint with 1, 2, 4, 8, 16 parallel streams.
2. **Interpret the curve:**
   - Throughput rises roughly linearly with streams, then flattens → causes A or B; the flat point is your line capacity and the starting concurrency.
   - Flat from 1 stream → cause D, or a hard server limit. Switch to the "reduce bytes" strategy (section 8) and warn the user honestly.
3. **Compare paths:** test direct endpoint vs accelerated or relay endpoint; pick the faster.
4. **Measure RTT and loss** to size chunks and choose protocol.

Show the user a plain result: "Your line can do about X MB/s. Estimated time: Y minutes."

### 4.2 Chunk planner

For S3-style multipart, the documented limits are: parts of 5 MiB to 5 GiB (no minimum for the last part), a maximum of 10,000 parts per upload. Plan like this:

```
part_size = clamp( ceil(file_size / 10000), 8 MiB, 64 MiB )
// round up to a multiple of 1 MiB
// for a 2 GB file: 8 to 32 MiB  →  ~64 to 256 parts
```

Rules of thumb:

- **Too small** → request overhead and per-request latency dominate.
- **Too large** → a failure wastes more work, and memory use grows. Peak memory ≈ `concurrency × part_size` (16 streams × 16 MiB ≈ 256 MiB).
- For slow or unstable lines, use smaller parts (4 to 8 MiB on providers that allow it) so a drop costs seconds, not minutes.
- Re-plan dynamically: the first chunks can be small (fast feedback), later ones larger once the connection is stable.

### 4.3 Pipeline

Three stages connected by bounded queues so the network never waits on disk or CPU:

```
Reader (async file I/O, read-ahead) ─► Hasher (CRC32C / SHA-256, parallel) ─► Uploader workers (N)
```

- Use memory-mapped or `pread` reads at offsets, so any chunk can be read independently and retried.
- Hash each chunk once, store in the manifest, and send it as the provider's checksum header where supported.
- Bounded queues give natural backpressure and cap memory.

### 4.4 Adaptive concurrency controller

This is the part that makes it "IDM-like" and smarter than fixed settings. Goal: find the number of streams that maximizes goodput without hurting the connection.

```
state: N = 4 (initial, or from probe), best = 0, step = 2
every T = 3 seconds:
    g = bytes_acked_in_window / T            // goodput
    err = error_or_timeout_rate
    if err > 5% or rtt_inflation > 2x baseline:
        N = max(1, floor(N * 0.7))           // multiplicative decrease
    else if g > best * 1.08:                 // meaningful improvement
        best = g; N = min(N + step, N_max)   // additive increase
    else if N > 1 and g < best * 0.9:
        N = N - 1                            // we overshot
    // otherwise hold N (plateau = line limit reached)
```

Details that matter:

- **Stop adding streams on plateau.** More streams past the line limit only add contention and unfairness.
- **Watch RTT inflation** (bufferbloat): if latency to the server climbs sharply, you are filling the router's queue and hurting the user's other traffic. Back off.
- **Cap by default** (e.g. N\_max = 16) with a user-visible "use all bandwidth / be gentle" setting. Optional bandwidth limiter (token bucket).
- **Warm-up:** new TCP connections start slow (slow start). Reuse connections across chunks rather than opening one per chunk.

### 4.5 Connection manager and transport

- **HTTP/1.1 pool:** one request per connection. The classic way to beat per-connection throttling, because each connection is a separate flow.
- **HTTP/2:** multiplexes many streams over one TCP connection. This is great for many small requests but **does not** bypass a per-connection throttle or TCP window limit. To get parallelism with H2, open several H2 connections.
- **HTTP/3 (QUIC), when the server supports it:** runs over UDP with its own flow control, so it avoids TCP window limits and handles high latency well. Research comparing H2 and H3 finds the advantages are largest with many short requests and high latency, and gains for bulk multi-stream transfer in a lab can be minimal. So treat it as an option to *benchmark*, not a guaranteed win.
- **Congestion control:** BBR (Linux 4.9+ kernels, plus QUIC stacks) typically beats loss-based CUBIC on high-latency, random-loss, or shallow-buffer paths; recent benchmarks report up to roughly 30% throughput gains in some topologies. It is not always better (for example Wi-Fi and some low-bandwidth cases), so the tool should expose it as an auto-tested option. On the client side you usually cannot change the OS algorithm per app, but you can on your relay servers, which is a strong reason for relay mode.
- **TLS:** reuse sessions, enable TLS 1.3 and session resumption to cut handshake cost.

### 4.6 Manifest and resume

A local SQLite database (WAL mode) per upload job:

```sql
CREATE TABLE job (
  id TEXT PRIMARY KEY, src_path TEXT, size INTEGER, mtime INTEGER,
  file_fingerprint TEXT,           -- hash of first/last MiB + size + mtime
  provider TEXT, dest TEXT, upload_id TEXT, part_size INTEGER, state TEXT
);
CREATE TABLE chunk (
  job_id TEXT, idx INTEGER, offset INTEGER, length INTEGER,
  checksum TEXT, status TEXT,      -- pending | inflight | done | failed
  etag TEXT, attempts INTEGER, PRIMARY KEY (job_id, idx)
);
```

Resume procedure after a crash or restart:

1. Check the source file's size and mtime are unchanged (else warn and restart).
2. Ask the provider what it already has (S3 `ListParts`, tus `HEAD`, Drive session status query) and **reconcile** with the local manifest. Trust the server over the local file.
3. Re-queue everything not confirmed.

### 4.7 Retry engine

- Retry each chunk independently; never restart the file.
- Exponential backoff with jitter: `delay = min(cap, base × 2^attempt) × random(0.5, 1.5)`.
- Classify errors: retry on timeouts, 5xx, 429/503 (honor `Retry-After`), connection reset. Do **not** retry on 4xx auth errors; refresh credentials or fail clearly.
- Per-chunk stall detector: if a chunk's rate is far below the median, **speculatively re-issue it on another connection** (hedged request) and keep whichever finishes first. This cuts tail latency from one bad connection.
- Global circuit breaker if the whole network is down: pause, poll connectivity, resume automatically.

### 4.8 Integrity

- Per-chunk checksum (CRC32C is fast; SHA-256 is stronger) sent with each part where the provider supports it.
- Final verification: provider-side ETag or checksum-of-checksums compared against the local manifest.
- Optionally re-read and verify the assembled object on a sample of ranges.

## 5. Provider plugin interface

```
interface Provider {
  capabilities(): { parallelChunks, maxParts, minPart, maxPart, resumable, checksum }
  init(job)            -> session            // e.g. CreateMultipartUpload / tus POST / Drive session URL
  uploadChunk(session, idx, bytes, checksum) -> receipt   // idempotent
  status(session)      -> list of confirmed chunks         // for resume
  complete(session, receipts)                              // e.g. CompleteMultipartUpload
  abort(session)                                           // free server storage
}
```

Chunk uploads must be **idempotent** so retries and hedged requests are safe.

| Provider | Mechanism | Parallel chunks? | Notes |
| --- | --- | --- | --- |
| S3 and compatibles (R2, B2, MinIO, Ceph) | Multipart upload | **Yes** | Best first target. Verify each vendor's limits separately; they are not all identical to AWS. |
| tus servers | PATCH with offsets; **Concatenation extension** for parallel partial uploads | Yes, if the server implements concatenation | Open protocol, resumable by design. Core tus is sequential per upload, so parallelism needs the extension. |
| Google Drive / OneDrive / Dropbox | Vendor session APIs | Mostly **sequential** per file | Parallelism often limited to ranges in order; gains come from transport tuning and resume rather than many streams. Check each API's current rules. |
| Your own server | Custom chunk protocol (or tus) | You decide | Best performance control. |

**Practical consequence:** the "many parallel streams" win is strongest on S3-style and your-own-server targets. For vendor drive APIs, set expectations accordingly.

## 6. Optional relay (edge ingest) service

Solves causes B and C when the client's path to the destination is slow.

```
Client ──(short hop, parallel chunks)──► Nearby relay ──(fast backbone)──► Destination
```

- Relay accepts chunks (tus-with-concatenation or a simple `PUT /upload/{id}/chunk/{n}` protocol), buffers on local disk, forwards via the destination's multipart API.
- Run it on a tuned Linux host: BBR, `fq` qdisc, large socket buffers, HTTP/3 enabled.
- Tradeoffs: you now operate infrastructure, handle user data, and pay bandwidth. Prefer provider-native acceleration first. For example, S3 Transfer Acceleration routes uploads to the nearest CloudFront edge and over AWS's backbone, and is billed per GB (roughly $0.04 to $0.08 per GB at the time of the sources reviewed), reportedly with no charge if it does not improve speed for a given upload. Measure before enabling.
- Security: end-to-end encrypt chunks client-side if the relay should not see content; short-lived signed upload tokens; per-job quotas.

## 7. Multi-path bonding (the one real way past a single line)

If the user has more than one uplink (Wi-Fi + phone tethering, two ISPs), the engine can **bind different workers to different network interfaces** (`SO_BINDTODEVICE` on Linux, interface-scoped sockets elsewhere) and spread chunks across them. Throughput approaches the sum of links. This works because chunked uploads are independent, and needs no special server support. It is a strong differentiator for the 3 KB/s case: two or three slow-but-independent links can still be far better than one.

## 8. When the line is the limit (cause D): reduce the bytes

If the probe shows the access line itself is the bottleneck, the engine should say so and offer:

1. **Compression** (zstd, level auto-tuned by CPU and bandwidth). Skip for already-compressed formats (mp4, jpg, zip) after a quick entropy sample.
2. **Delta / dedupe:** content-defined chunking (FastCDC-style) with a chunk-hash index so only new or changed data is sent when a previous version exists at the destination.
3. **Re-encode media** (e.g. H.265 or AV1 for video) with a clear quality warning.
4. **Switch link** (mobile data, another network) using the multi-path feature.
5. **Sneakernet fallback:** for 2 GB at 3 KB/s, copying to a USB drive or SD card is faster by orders of magnitude, so say so.

## 9. Security

- Never embed long-lived provider keys in the client. Use **presigned URLs or short-lived credentials** minted by your backend or the user's own login (OAuth for drive APIs).
- TLS 1.3 everywhere; certificate validation on; no silent downgrade.
- Manifest contains paths and checksums, not secrets. Store tokens in the OS keychain.
- Abort incomplete multipart uploads on cancel, and recommend a bucket lifecycle rule to expire abandoned ones (they cost storage).
- Rate-limit and fingerprint relay clients to prevent abuse.

## 10. Technology choices

| Layer | Recommendation | Why |
| --- | --- | --- |
| Engine | **Rust** (tokio, hyper/reqwest, quinn for QUIC) or **Go** | Strong concurrency, small static binaries, easy cross-compile |
| Manifest | SQLite (WAL) | Crash-safe, zero setup |
| Desktop UI | Tauri (Rust) or Electron | IDM-like look; Tauri is lighter |
| Browser integration | Extension that hands off a file picker or drag-drop to the local engine (native messaging) | Browsers cannot bind NICs or do raw tuning |
| Pure web variant | Web Workers + `fetch` with `File.slice()` and presigned multipart URLs | Works without install; limited to HTTP/2/3 and browser connection caps |
| CLI | Same engine, thin wrapper | Scriptable, easy to benchmark |

Browser caveat: browsers cap connections per host (HTTP/1.1 is typically around 6), so a web-only version needs HTTP/2 or HTTP/3 and multiple hostnames or endpoints to approach native performance.

## 11. Testing and benchmarking

1. **Network emulation:** `tc netem` or Toxiproxy to simulate latency (50 to 300 ms), loss (0 to 5%), bandwidth caps, **per-connection caps**, and random disconnects.
2. **Key scenarios:**
   - Per-connection cap of 50 KB/s on a 50 Mbps line: expect near-linear scaling until the line saturates.
   - Total cap of 3 KB/s: expect no scaling and a correct "line-limited" diagnosis.
   - Kill the process mid-upload at random points: expect resume with at most one chunk repeated per worker.
   - Flip network off/on: expect automatic pause and resume.
3. **Baselines to beat:** `aws s3 cp` (with CRT client enabled), `rclone copy` with tuned chunk size and concurrency, tus-js-client with parallel uploads.
4. **Metrics:** goodput vs line capacity (target ≥ 90% when the line is the limit), time to first byte of payload, resume overhead, CPU and memory.

## 12. Expected results (realistic)

| Situation | Today | With this tool |
| --- | --- | --- |
| Per-connection throttle 50 KB/s, line 20 Mbps (≈2.5 MB/s) | \~11 hours | \~15 min (≈50 streams equivalent, capped by line) |
| High-latency path, single-stream 300 KB/s, line 50 Mbps | \~1.9 hours | \~5 to 6 min |
| Line truly 3 KB/s | \~8 days | Still \~8 days; compression, delta, extra links, or physical transfer are the only levers |

The first two are illustrative calculations, not measurements; actual results depend on the path.

## 13. Roadmap

- **Phase 1 (MVP, 2 to 4 weeks):** Rust or Go CLI, S3 multipart provider, fixed concurrency, manifest-based resume, per-chunk retry, integrity checks.
- **Phase 2:** Probe and diagnose, adaptive concurrency controller, progress UI (speed, ETA, per-stream view), bandwidth limiter.
- **Phase 3:** tus provider with concatenation, accelerated endpoints, hedged requests, HTTP/3 option.
- **Phase 4:** Drive/OneDrive/Dropbox plugins, desktop app, browser extension.
- **Phase 5:** Relay service, multi-path bonding, compression and delta engine.

## 14. Risks and open questions

- **Server cooperation** is the biggest limit; many sites expose no chunked upload API.
- **Fairness and abuse:** aggressive parallelism can violate provider terms or hurt other users on the connection. Ship sensible caps and honor `429`/`Retry-After`.
- **Vendor API changes** (Drive, OneDrive, Dropbox) require ongoing maintenance.
- **Relay cost and trust** if you operate infrastructure.
- **Competition:** rclone, `aws s3 cp` with CRT, and tus clients already do parts of this. Your value is the *diagnosis, adaptive tuning, multi-path, and friendly UX* in one tool.

## 15. Sources reviewed

- AWS S3 multipart upload limits: https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html
- S3 multipart part sizing and rclone settings: https://portal.mlcloud.uni-tuebingen.de/user-guide/tutorials/s3-multipart-user-guide/
- tus protocol: https://tus.io/protocols/resumable-upload and https://github.com/tus/tus-resumable-upload-protocol
- tus parallel uploads via Concatenation (implementation example): https://pypi.org/project/resumable-upload/
- BBR vs CUBIC: https://arxiv.org/html/2510.22461v1 and https://www.ietf.org/archive/id/draft-ietf-ccwg-bbr-06.html
- HTTP/3 vs HTTP/1.1 vs HTTP/2 study: https://www.vestnik-donstu.ru/jour/article/view/2670
- S3 Transfer Acceleration: https://oneuptime.com/blog/post/2026-02-12-enable-s3-transfer-acceleration-faster-uploads/view
- AWS CRT client for the CLI: https://repost.aws/knowledge-center/s3-upload-large-files

*Note: details for Google Drive, OneDrive, and Dropbox upload APIs come from general knowledge rather than the sources above; verify current rules before building those plugins.*
