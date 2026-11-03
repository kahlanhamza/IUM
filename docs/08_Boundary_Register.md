# IUM — Boundary Register

> Created 2026-11-03 (#31). The owner's rule: *"we must understand the relationship between
> entities and know what is going to break."* This file is the working answer. It is a
> checkable artifact, not prose: every claim in it cites either a defect number or a test
> that can be run today.

## 1. The forensic verdict (what "fix → break" actually was)

Every issue born during the stress campaign (#23–#30) was checked against the tree that
preceded its cause:

| Born of | Defects surfaced | Caused by the previous fix? |
|---|---|---|
| #22 real-world drill | #23 #24 #25 #26 | No — latent, first exposure to real server + impaired wire |
| #26/#27 fixes | #28's honest-mode requirement | No — the drill could not see resplit; visibility gap, not breakage |
| #28 worse-case ladder | #28's idle-timeout killer, #29 races | No — first exposure to B/s links and runner timing |
| #29 determinism fixes | #30 flush race | No — A/B proof: identical distribution at pre-#27 `93ff6ba` and pre-#28 `5ca7f99` |
| Ungated commits (process slips) | defects 8–10, #11, two pipe-masked gate exits | **Yes — the only true regressions**, all process, each institutionalized a guard |

**The rule this register encodes:** entities never break — *edges* break, under timing or
semantics owned by the other side (server, network, runner, scheduler). A new probe class
exposes the next seam; it does not create it.

## 2. The entity map (re-derive it any session)

```bash
rg -o "use crate::[a-z_]+" ium-core/src/*.rs | sort -u
```

Verified 2026-11-03 at `812188f`:

```
supervisor → manager ─────┬→ journal ←→ chunk (two-way)
                          ├→ eviction      (leaf — pure policy, no intra-crate deps)
                          ├→ retry         (leaf — pure policy)
                          ├→ state         (leaf — pure)
                          └→ adapter trait ← { adapter_http, adapter_s3 → sigv4, adapter_tus }
                                 (s3/tus also reuse adapter_http's streaming plumbing)
```

`manager.rs` is the hub: every orchestration seam below passes through it. `journal.rs` is
the truth every seam reads and writes (ADR-002).

## 3. The seam table — every defect to date, mapped to its edge

| Seam (edge) | What it owns | Defects that lived there | Pinned by (run today) |
|---|---|---|---|
| **journal ↔ everything** (state truth) | schema migrations, loud writes, rows-affected | Sprint-0 #1–#5, #15 unknown-state resurrection | `migrations_are_versioned_and_reopen_safe`, exhaustive `parse_state`, fuzz (`nightly.yml`) |
| **manager ↔ journal** (transitions) | journaled-before-execute, cancel racing any transition | #26 Finishing refused Cancel | `tests/finishing_cancel.rs` |
| **manager ↔ chunk pool** | leases, re-rentable partial confirms, plans, resplit reshape | Sprint-0 #4, #5 planless Done, #27 orphaned policy, #28 frozen rule | `tests/adaptive_resplit.rs`, `tests/frozen_link_resplit.rs`, fuzz invariants |
| **manager ↔ eviction** | window medians, strikes, lone-vote floor | #25 lone impaired link | `tests/lone_worker_eviction.rs`, #18 live drill, #32 `s7net` (RTT link must never be struck; loss stalls recorded honestly) |
| **manager ↔ adapter session** | probe token replay, reattach, budgets | #12 Finishing-entry empty session map | #12 5×RED pins, kill-9 drills (s3/tus) |
| **manager ↔ retry** | counted attempts, backoff, exhaustion → Failed | Sprint-0 #5 infinite retry | attempt-map tests, #32 `s7net[reset]` (a real mid-body RST must land as ONE counted `chunk_error {kind: Transient, action: Retry}`) |
| **adapter trait contract** | required methods, `ProbeReport` declarations | #1 verify-default, #8 cap ignored, #13 max_parallel ignored | trybuild `verify_is_required.rs`, `plan_capped` RED, `max_parallel` offline RED |
| **adapter_s3 ↔ server wire** | etag case/entities, duplicate PUTs, Complete idempotency, required headers | `&#34;`/`&quot;` (`InvalidPart`), #24 duplicate entry, #36 zero-byte PutObject without `Content-Length` (411 on rclone/gofakes3 — AWS-real semantics SeaweedFS masked) | SeaweedFS + Garage vectors pinned as tests, `tests/verify_repair.rs`; **#33: the full 23-test suite now runs against THREE implementations (SeaweedFS 4.51, Garage rehearsal, rclone serve s3 1.75.2) — ListParts-works-despite-docs and `&#34;` numeric entities re-confirmed live on rclone** |
| **adapter_tus ↔ server wire** | offset semantics, 409 realign, flush scheduling, extension surface | #10 409 drift, #30 flush race; **#33: tusd's GET-download is NOT core — @tus/server answers GET with 404 (read-back tests loud-skip there)** | live suite on BOTH lanes (`IUM_TUS_SERVER=tusd|tus-node`; 12/12 vs tusd, 11+1-loud-skip vs @tus/server 2.4.7), `WIRE − WASTED == missing` exact assert |
| **harness/proxy ↔ reality** | throttle model, wedge determinism, server idiosyncrasies, network impairment models, server diversity lanes | #23 NameError, #28 64 KB-burst vs 30 s idle timeout, #29 link-speed races | the S1–S7 scenario asserts + self-diagnosing REDs (#32 `s7net`: loss/RTT/RST models, wire-truth lines LOSS/RESET-WIRE); #33 lanes: `s3-test.sh --server rclone` (SHA256SUMS guard), `tus-test.sh --server tus-node` (npm lockfile guard) |
| **process** (gates, tree state) | what "green" measures | defects 8–10, #11, pipe-masked exits | `gate.sh` committed-tree + pipefail rule |

## 4. How to use it (mandatory since #31)

Every non-trivial issue's design note names:

1. **Entities touched** — from the map in §2;
2. **Edges at risk** — rows in §3 the change could disturb;
3. **The pinning test per edge** — an existing test cited by name, or a new one *in the same
   issue*. "Will add later" is not a pinning test.

Example (the shape, from #30): entities = `adapter_tus`, manager accounting; edges at risk =
adapter_tus ↔ server wire (flush scheduling), manager ↔ eviction (`moved()` semantics);
pinning = `moved() − wasted() == tail` offline + `WIRE − WASTED == missing` in the drill.

## 5. Predicting the next break

When a new probe class arrives, ask which §3 rows it exercises **for the first time**:

- a new transport → the adapter trait row + its own wire row (expect etag/offset-style surprises);
- a new impairment (loss, latency, reset) → the eviction + retry rows (windows were tuned on bandwidth stalls);
- a new scale (hours, GBs, disk-full) → the journal rows (WAL growth, ENOSPC while writing truth);
- a new surface (the real shell) → every row, untested end-to-end.

Those four lines are exactly the M6 probe generation (#32–#35) — filed the same day this
register landed.

## 6. Maintenance rule

Any commit that changes an edge (new caller, new policy input, new adapter, new harness
model) updates the affected §3 row **in the same commit**. The changelog checkbox covers
behavior; this register covers coupling. A stale register is a defect — file it as `process`.
