#!/usr/bin/env python3
"""File issue #98: the OS life-event lane — the SIGSTOP freeze drill."""
import json, urllib.request, pathlib

TOKEN = pathlib.Path(".secrets/gh_token").read_text().strip()

title = "The OS life-event lane: SIGSTOP/SIGCONT freeze mid-body — counted transients, never a false terminal, honest Done"

body = """## Why this issue exists

The release-prep audit (2026-11-10) named the gap: the probes torture the *wire*
(loss, RTT, RST — #32) but nothing simulates the *laptop*: the user closes the lid
mid-upload. On wake, every in-flight socket has been idle far past the server's
patience, wall-clock backoff timers appear expired, and the engine must treat the
whole event as counted transient work — never a silent stall, never a false
terminal Fail, never re-sent confirmed bytes.

## Design note (the honest in-process twin of sleep)

`SIGSTOP`/`SIGCONT` on the real engine child process is exactly what lid-close
does to a process: ALL threads freeze (tokio timers, journal writer, in-flight
PUTs), the wall clock keeps running, and the far side of every socket keeps
aging. The kill-9 drill machinery (#9) is reused wholesale — same child binary,
same harness, same journal-truth discipline — with the drill shape changed:

1. **Determinism**: the child gains a `--freeze-after-parts N --freeze-threshold B`
   mode — after N completed part calls, the next starting call becomes the
   *watched call*: it runs the real inner upload and, once `ProgressReporter`
   observes ≥ B bytes genuinely in flight, prints `INFLIGHT <bytes>`. The parent
   SIGSTOPs on that line — the freeze provably lands mid-body (bytes on the
   socket), not in a scheduler gap.
2. **The freeze**: 35 s — safely past SeaweedFS's measured 30 s idle timeout
   (the #95 saga's volume-server fact), so the server kills the frozen PUTs.
3. **The wake**: SIGCONT; the dead attempts must return as counted transients.
4. **The pins** (parent asserts, all journal/wire truth):
   - the child reaches `DONE done` — the wake never becomes a false terminal;
   - at least ONE `chunk_error` event journaled AFTER the freeze timestamp —
     the death was counted, not silent;
   - the child's wire counter is bounded: `WIRE ≤ SIZE + 2×PART` — the two
     frozen workers may waste at most their own parts (physics, not honesty);
   - the job state path never prints `FAILED`.
5. **Where it runs**: `scripts/s3-test.sh --with-freeze` (SeaweedFS-canonical,
   same as the kill-9 drill), a second step in the nightly `s3_drill` job
   (CI-economics rule: same job, no new download lanes). Linux-only with a
   loud skip elsewhere — the SIGSTOP contract. Parent signals via the system
   `kill` binary — **zero new Rust dependencies** (std cannot signal, and the
   dependency iron rule stands).

## Acceptance

- [ ] Child freeze mode lands mid-body deterministically (INFLIGHT line).
- [ ] Parent asserts: Done after wake · counted transient post-freeze · wire
      bounded · no FAILED.
- [ ] `--with-freeze` harness flag + nightly step; local green vs SeaweedFS.
- [ ] CI green; no new runtime or dev dependencies.
"""

req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues",
    data=json.dumps({"title": title, "body": body, "labels": []}).encode(),
    headers={"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
             "Content-Type": "application/json"},
    method="POST")
with urllib.request.urlopen(req) as r:
    d = json.load(r)
    print(f"filed #{d['number']}: {d['title']}")
    print(d["html_url"])
