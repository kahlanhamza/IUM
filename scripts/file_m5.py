#!/usr/bin/env python3
"""File milestone M5 + the API-freeze & real-data-evidence tranche for IUM.

Reads the PAT from /home/z/my-project/.secrets/gh_token (never printed).
Idempotent: existing OPEN issues are matched by exact title and skipped;
the milestone is matched by title.

Usage: python3 scripts/file_m5.py
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN_PATH = Path("/home/z/my-project/.secrets/gh_token")

MILESTONE = {
    "title": "M5: API freeze & real-data evidence",
    "description": "docs/04_Roadmap.md M5 + the owner's standing mandate (2026-11-02): "
    "the public API freezes (docs complete, missing_docs enforced, cargo doc clean) and "
    "semver + CHANGELOG discipline lands. Folded in per owner instruction: every claim "
    "must be tested on REAL data — a real video file through all three transports, and "
    "slow-connection eviction proven against a genuinely throttled connection, not "
    "simulated wedges. DoD: every issue landed failing-test-first, gates green locally "
    "+ 3-OS CI, live evidence posted on the issues.",
}

ISSUES = [
    {
        "title": "core: real-data e2e drill — a real video file through HTTP + TUS + S3, byte-verified",
        "labels": ["tests", "core"],
        "body": (
            "## Problem (owner mandate 2026-11-02: always test on real data)\n"
            "Survey at HEAD `1a8be4c`: EVERY fixture in the repo uploads zeros — "
            "`vec![0u8; N]` in `integration.rs`, `supervisor.rs`, `http_one_shot.rs`, "
            "`adapter_tus.rs`, `write_zeros()` in `s3_drill.rs`, zeros blocks in "
            "`tus_drill.rs`. No real file has EVER gone through the engine. Zeros are "
            "maximally compressible and trivially patterned; the engines' streaming "
            "paths, hashes, etags and byte-fidelity claims deserve evidence over real "
            "content — an actual video, as the owner asked for.\n\n"
            "## Design\n"
            "- Fixture generator `scripts/make_real_fixture.sh`: ffmpeg "
            "(testsrc2 + sine audio, H.264/AAC MP4, ~20-40 MiB) → a REAL playable "
            "video. The file is generated at test time, never committed (binaries are "
            "not repo evidence, ADR-009). If ffmpeg is absent the suite skips LOUDLY "
            "(env-gated honesty contract, same as the S3/TUS live suites) — never "
            "silently fakes with zeros.\n"
            "- The invariant is END-TO-END FIDELITY, not a pinned hash: "
            "`sha256(received) == sha256(source file as generated)` in that "
            "environment. Playability asserted via ffprobe (container + codec streams "
            "intact after upload — where verifiable; ffprobe-missing → sha-only, "
            "recorded loudly).\n"
            "- Three e2e tests (env-gated like the existing live suites): HTTP "
            "loopback server, TUS vs pinned tusd (`IUM_TUS_ENDPOINT` from "
            "`scripts/tus-test.sh`), S3 multipart vs pinned SeaweedFS "
            "(`IUM_S3_ENDPOINT` from `scripts/s3-test.sh`) — the real video through "
            "each adapter, journal reaches Done only after the adapter's own verify, "
            "downloaded/received bytes hash-equal the source.\n"
            "- Multi-chunk across the video's real byte boundaries (fixture sized "
            "above the default chunk size so several chunks+parts move; for S3, "
            "parts land in arbitrary order — the real content makes the "
            "assemble-in-order verify meaningful).\n\n"
            "## Acceptance criteria\n"
            "- RED: the survey itself is the RED state — no real-content test exists "
            "at HEAD (pasted evidence). The drill tests land green with live output "
            "(sha256 pairs + ffprobe verdict) posted on this issue.\n"
            "- `cargo test -p ium-core` includes the drill when endpoints+ffmpeg "
            "exist; skips loud otherwise; 126 existing tests untouched and green.\n"
            "- Harness wiring: `scripts/s3-test.sh`/`tus-test.sh` grow a "
            "real-fixture pass (or the drill tests reuse their endpoints) without "
            "breaking the 3-OS CI lanes.\n\n"
            "## Refs\n"
            "Owner standing mandate (2026-11-02); ADR-009 (evidence discipline); "
            "#7 #9 #10 (the three transports); docs/04 M5 amendment."
        ),
    },
    {
        "title": "core: real slow-connection drill — throttled server, eviction fires on live wire throughput",
        "labels": ["tests", "core"],
        "body": (
            "## Problem (owner mandate 2026-11-02: real slow-connection testing)\n"
            "`eviction.rs` implements spec §4.2 (10 s windows, 0.4×median floor, "
            "evict on 2nd consecutive strike) but the ONLY tests driving it are "
            "simulated: `ScriptedAdapter` wedges, `chunk_delay_ms` sleeps, 40 ms "
            "windows in `integration.rs::stalled_chunk_is_evicted_and_job_still_"
            "completes`. A genuinely SLOW connection — real TCP, real throttling, "
            "real measured throughput — has never been evicted. The floor is "
            "relative to the pool median, so a throttled worker among healthy "
            "peers must be evicted mid-flight on live evidence, its chunk "
            "re-leased, and the job must still complete honestly.\n\n"
            "## Design\n"
            "- Real throttled loopback HTTP server (test code): token-bucket "
            "rate-limit per connection (e.g. ~64 KiB/s vs unthrottled peers) — the "
            "slowness lives in the SOCKET, not in a test clock.\n"
            "- Multi-chunk job (≥3 workers), one worker pinned to the throttled "
            "route; real `EvictionPolicy` with test-scaled window/strikes "
            "(structure unchanged) and throughput measured the way production "
            "measures it: bytes_confirmed deltas over real elapsed time.\n"
            "- Assert, from journal truth: the slow lease is EVICTED mid-flight "
            "(eviction observable via the journal/snapshot, not a mock hook); the "
            "chunk is re-leased and completes via a healthy worker; the job ends "
            "Done; the server received every byte of every chunk exactly-once per "
            "confirmed span; the evicted worker's partial bytes are reconciled by "
            "the existing re-rentable partial-confirm path (#2-era invariant).\n"
            "- The unthrottled peers must NOT be evicted (floor is relative to "
            "median — pins that a slow minority dies, not the majority).\n"
            "- Sizing: total runtime bounded (seconds, not minutes) so it can live "
            "in the standard suite; if it cannot be bounded honestly it becomes "
            "env-gated like the drills — decided by measurement, recorded on the "
            "issue.\n\n"
            "## Acceptance criteria\n"
            "- RED at HEAD if eviction does not fire on real wire evidence (test "
            "proves the gap); if it fires, the live numbers (measured B/s per "
            "worker, strike windows, eviction event) are posted on this issue as "
            "the evidence — same honesty as #11's first-attempt green.\n"
            "- Existing simulated eviction tests stay green and untouched.\n"
            "- fmt + clippy `-D warnings` + full gates green on the committed tree.\n\n"
            "## Refs\n"
            "Owner standing mandate (2026-11-02); spec §4.2; eviction.rs; "
            "integration.rs stall test; #2 (re-rentable partial confirms); "
            "docs/04 M5 amendment."
        ),
    },
    {
        "title": "core: API freeze — public surface reviewed, missing_docs enforced, cargo doc clean",
        "labels": ["documentation", "core"],
        "body": (
            "## Problem\n"
            "docs/04 M5 box 1: `ium-core` public API reviewed and documented, "
            "`cargo doc` clean. The public surface grew issue-by-issue (M1→M4) "
            "without a documentation gate: `lib.rs` re-exports manager/journal/"
            "chunk/retry/eviction/adapter/supervisor/state/ids, but nothing "
            "enforces docs on public items, and `cargo doc` has never been checked "
            "in a gate or CI.\n\n"
            "## Design\n"
            "- Surface review FIRST: decide what is genuinely public (the facade "
            "ium-app uses: JobManager, Supervisor, JobSpec, JobState, snapshot "
            "types, TransferAdapter + ProbeReport/adapter registry, EvictionPolicy) "
            "vs internals that must become `pub(crate)` (journal internals, ids "
            "plumbing, whatever the review exposes). Shrinking the surface is part "
            "of the freeze — semver hurts less before 1.0 the smaller the API.\n"
            "- `#![deny(missing_docs)]` (or `missing_docs = warn` + CI `-D "
            "warnings` equivalent — pick the form that cannot be bypassed) on the "
            "crate; every public item gets a real doc comment (what it guarantees, "
            "not what it is); crate-level doc in lib.rs = the 30-second architecture "
            "map with invariants (journal-is-truth, verify-before-declare, counted "
            "retries) linked to the ADRs.\n"
            "- `cargo doc --no-deps` with warnings-as-errors added to gate.sh and "
            "CI (cheap lane, no extra toolchain).\n"
            "- Contract trybuild tests unchanged; no behavior change — docs + "
            "visibility only, so the 126-test suite is the regression net.\n\n"
            "## Acceptance criteria\n"
            "- RED: `cargo doc` (or the missing_docs lint) fails at HEAD with the "
            "list of undocumented public items — pasted on the issue.\n"
            "- After fix: `cargo doc --no-deps` zero warnings locally AND in CI; "
            "missing_docs enforced permanently; the reviewed-and-shrunk surface "
            "recorded on the issue (what became private, why).\n"
            "- Full gates green on the committed tree; no test churn beyond "
            "visibility fixes.\n\n"
            "## Refs\n"
            "docs/04 M5; ADR-002/004 invariants to document; #15 (Supervisor is "
            "the newest facade — review it too); blocks #20 (versioning freezes "
            "the reviewed surface)."
        ),
    },
    {
        "title": "repo: semver versioning policy + CHANGELOG.md discipline",
        "labels": ["documentation", "process"],
        "body": (
            "## Problem\n"
            "docs/04 M5 box 2: versioning policy (semver), changelog discipline. "
            "Today `ium-core` carries a placeholder version, there is no "
            "CHANGELOG.md, and six milestones of user-visible change (defect fixes, "
            "trait contract changes, two real transports, the app shell) live only "
            "in git log + STATUS.md — unrecoverable for a downstream consumer.\n\n"
            "## Design\n"
            "- Versioning policy section in docs/05_Engineering_Process.md: semver "
            "with 0.x pre-1.0 rules (0.MINOR may break, 0.PATCH is fixes only); "
            "workspace members versioned together (`ium-core`, `ium-app/src-tauri`) "
            "while the facade is coupled; breaking-change rule = a compile-fail "
            "contract test must pin the break (the trybuild discipline already in "
            "use) before the MINOR bump.\n"
            "- `CHANGELOG.md` (Keep a Changelog format) seeded from the verified "
            "record: Unreleased section + entries for the shipped work (M1 engine "
            "correctness, M2 hardening/verify-required, M3 S3+TUS transports + "
            "kill-9 drills, M4 shell + Supervisor + UI slice), each tied to its "
            "issue numbers. Entries cite evidence (issue/commit), never prose "
            "promises.\n"
            "- Rule going forward: every issue that changes public behavior edits "
            "CHANGELOG.md in the same commit (checked by review, enforced by the "
            "issue template checklist).\n"
            "- Version bump executed as part of this issue: `ium-core` 0.1.0 as "
            "the first frozen surface (post-#19 review), recorded in the "
            "changelog.\n\n"
            "## Acceptance criteria\n"
            "- CHANGELOG.md exists with real entries per milestone, all issue "
            "numbers verifiable against the tracker.\n"
            "- Policy section merged into docs/05; issue template gains the "
            "changelog checkbox.\n"
            "- Workspace versions consistent; `cargo check` green; gates green on "
            "the committed tree.\n\n"
            "## Refs\n"
            "docs/04 M5; docs/05 (process home for the policy); #19 (frozen "
            "surface is what gets versioned); ADR-007 (gates unchanged)."
        ),
    },
]


def api(method: str, path: str, payload: dict | None = None) -> dict:
    url = f"https://api.github.com/repos/{REPO}{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {TOKEN_PATH.read_text().strip()}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-file-m5",
        },
    )
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read() or b"{}")


def main() -> int:
    milestones = api("GET", "/milestones?state=all&per_page=100")
    ms = next((m for m in milestones if m["title"] == MILESTONE["title"]), None)
    if ms is None:
        ms = api("POST", "/milestones", MILESTONE)
        print(f"milestone created: #{ms['number']} {ms['title']}")
    else:
        print(f"milestone exists:  #{ms['number']} {ms['title']}")
    msn = ms["number"]

    open_issues = api("GET", "/issues?state=open&per_page=100")
    titles = {i["title"] for i in open_issues}
    for spec in ISSUES:
        if spec["title"] in titles:
            print(f"issue exists:   {spec['title']}")
            continue
        body = spec["body"] + f"\n\n---\n_Milestone: M5: API freeze & real-data evidence_"
        issue = api(
            "POST",
            "/issues",
            {
                "title": spec["title"],
                "body": body,
                "labels": spec["labels"],
                "milestone": msn,
            },
        )
        print(f"issue filed:    #{issue['number']} {issue['title']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
