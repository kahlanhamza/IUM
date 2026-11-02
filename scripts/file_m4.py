#!/usr/bin/env python3
"""File milestone M4 + the app-shell tranche issues (#13-#16) for IUM.

Reads the PAT from /home/z/my-project/.secrets/gh_token (never printed).
Idempotent: existing OPEN issues are matched by exact title and skipped;
the milestone is matched by title.

Usage: python3 scripts/file_m4.py
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN_PATH = Path("/home/z/my-project/.secrets/gh_token")

MILESTONE = {
    "title": "M4: App shell",
    "description": "docs/04_Roadmap.md M4 — the frozen shell (ADR-006) unlocks now that "
    "M2+M3 are done. Prerequisite correctness fix (#13, TUS sequential planning) rides "
    "first; then the shell compiles on 3 OS (#14), the engine actually uploads through "
    "it (#15), and the Sprint-1 UI slice lands (#16, spec §8). DoD: every issue landed "
    "failing-test-first, gates green locally + 3-OS CI, honest badges end to end.",
}

ISSUES = [
    {
        "title": "core: ProbeReport.max_parallel — adapter declares concurrency, planner obeys (TUS ⇒ 1)",
        "labels": ["core", "enhancement"],
        "body": (
            "## Problem (loop finding, recorded in STATUS §3 at #11 close)\n"
            "TUS is ONE sequential-offset resource, but `create_job` gives every RESUMABLE "
            "adapter `max_conns` concurrent workers (`manager.rs`: `workers = if resumable "
            "{ max_conns } else { 1 }`). With max_conns > 1 a TUS job leases chunk k+1 and "
            "PATCHes it at its absolute `[begin, end)` while chunk k is still in flight → "
            "server offset ≠ lease.resume_from → 409 → realign → when the drift exceeds "
            "what realign can absorb, `Unresumable`. The #11 drill only survived because "
            "it ordered chunks locally — a manager-driven concurrent TUS job is broken by "
            "construction today.\n\n"
            "## Design (extends ADR-011's principle: adapter declares, planner obeys)\n"
            "- `ProbeReport` gains `max_parallel: Option<u32>` (`None` = no declared "
            "limit). This is the SAME declaration channel as `max_chunk_bytes` (#8) — no "
            "per-adapter special cases in the manager.\n"
            "- `adapter_tus::probe` declares `Some(1)`. S3/HTTP declare `None` (S3 parts "
            "are independent; HTTP is one-shot and already planned with 1 worker).\n"
            "- Manager: after probe, the drive round spawns "
            "`min(planned_workers, max_parallel.unwrap(u32::MAX))` workers. No re-plan is "
            "needed: the lease order is lowest-idx-first, so a single worker walks TUS "
            "chunks in ascending offset order and every PATCH starts exactly at the "
            "server's offset. Multi-chunk TUS plans stay valid; the concurrency was the "
            "bug, not the chunking.\n"
            "- The clamp is journaled like every probe-boundary decision (`replan_cap` "
            "already covers the chunk cap; the parallel clamp is a drive-round property, "
            "re-derived from the journal probe event on every resume — no new event "
            "needed because workers-per-round is not persisted state).\n"
            "- Eviction/tail-race semantics unchanged: a 1-worker pool is the existing "
            "lone-worker case (never evicted).\n\n"
            "## Acceptance criteria\n"
            "- RED (offline, deterministic): scripted adapter declares "
            "`max_parallel=Some(1)`; chunk 0's upload call is wedged at entry; with "
            "max_conns=4 the current code leases chunk 1 concurrently — test asserts NO "
            "second lease exists while chunk 0 is wedged and fails at current HEAD.\n"
            "- RED (live, env-gated `IUM_TUS_ENDPOINT`): a 16 MiB TUS job with "
            "max_conns=4 — assert Done + `Σ bytes_confirmed == size` after the fix.\n"
            "- `None` declarations are byte-identical in behavior to today (no clamp).\n"
            "- trybuild/contract surface untouched; fmt + clippy `-D warnings` + full "
            "gates green on the committed tree.\n\n"
            "## Refs\n"
            "ADR-011 (max_chunk_bytes precedent); STATUS §3 finding; #8; #10; #11. "
            "Blocks #15 (the shell must not be able to create broken TUS jobs)."
        ),
    },
    {
        "title": "app: unfreeze the shell — ium-app compiles and builds green on 3 OS (ADR-006 amendment)",
        "labels": ["ci", "enhancement"],
        "body": (
            "## Problem\n"
            "ADR-006 froze `ium-app/` (Tauri 2 + React) until M2+M3. Both are done "
            "(#1–#12 closed, 119/119 gates). The shell has never been compiled in this "
            "repo's history — `package-lock.json` doesn't exist, the Tauri crate was "
            "never clippy'd, and the Sprint-0 IPC contract (`src-tauri/src/lib.rs`) is "
            "unverified by any gate.\n\n"
            "## Design\n"
            "- ADR-006 amended: the freeze is lifted; the shell joins the NON-bypassable "
            "gates with its own lane: `npm ci && npm run build` (tsc strict + vite "
            "build) and `cargo clippy -p ium-app --all-targets -- -D warnings` + `cargo "
            "test -p ium-app` (src-tauri joins the workspace as a member so the engine "
            "facade types are shared and the lockfile is one).\n"
            "- `gate.sh` gains the app lane AFTER the engine lane; npm build runs before "
            "cargo so `tauri::generate_context!` finds `dist/`. Engine gates stay "
            "exactly as they are — no weakening (ADR-007).\n"
            "- Linux system deps (webkit2gtk-4.1, gtk3) installed via apt in CI and "
            "documented for the sandbox; Windows/macOS need none (wry/tao native).\n"
            "- Tauri codegen needs bundle icons: generate a minimal honest icon set "
            "(one source PNG → 32/128/128@2x png + .ico) via script, committed.\n"
            "- `package-lock.json` committed; `node_modules` + `dist` gitignored.\n\n"
            "## Acceptance criteria\n"
            "- `bash scripts/gate.sh` runs BOTH lanes on the committed tree and is green.\n"
            "- CI 3-OS matrix gains an `ium-app` job (ubuntu: apt deps; windows/macos: "
            "plain cargo) — green on the tranche's commits.\n"
            "- `npm run build` produces `dist/` consumed by the tauri context; clippy "
            "clean with zero allow-escapes.\n"
            "- ADR-006 status updated in docs/06; roadmap M4 checkbox progress noted.\n\n"
            "## Refs\n"
            "ADR-006 (amendment), ADR-007 (gates), docs/04 M4; blocked-by nothing; "
            "blocks #15/#16."
        ),
    },
    {
        "title": "app: runtime wiring — the shell actually uploads (run_job spawn, job_updated events, cancel)",
        "labels": ["enhancement"],
        "body": (
            "## Problem\n"
            "The Sprint-0 shell forwards commands to `JobManager` but NOTHING ever calls "
            "`run_job` — creating a job in the UI would park it in Created forever. Spec "
            "§8/Sprint-1 item 9 requires commands `create_job`, `pause`, `resume`, "
            "`cancel`, `inspect_chunks` and `job_updated` events. `cancel` does not "
            "exist in the engine (state machine has no Canceled state).\n\n"
            "## Design\n"
            "- `Engine` state becomes `{ manager: Mutex<JobManager>, tasks: Mutex<HashMap<job_id, JoinHandle>> }`. "
            "`create_job` / `resume_signal` spawn `tauri::async_runtime::spawn(run_job)`; "
            "the watcher re-spawns on resume after Paused.\n"
            "- `job_updated` events: a per-active-job emitter task polls `snapshot()` on "
            "a fixed interval (journal-truth; the UI NEVER computes transfer state — "
            "spec §2.3) and emits the snapshot; terminal states emit once and stop.\n"
            "- `cancel` in the engine: honest terminal state `Canceled` — state.rs "
            "variant + journal migration (`PRAGMA user_version` bump) + transition rules "
            "(legal from Created/Probing/Running/Paused) + manager `cancel()` that "
            "aborts the drive task and journals Canceled BEFORE aborting (ADR-002: "
            "transitions journaled before they execute). Failed stays Failed — a user "
            "cancel must not lie as an error.\n"
            "- Commands: `stat_file(path) -> {size}` (create-job dialog needs the size "
            "the engine requires in JobSpec; the shell stats, the engine plans), "
            "`cancel_job`, `inspect_chunks` = existing `snapshot` aliased for the UI "
            "vocabulary; api.ts updated to the same names.\n"
            "- The Engine layer (manager spawn/cancel/emitter logic) is factored to be "
            "unit-testable WITHOUT a webview: tauri glue stays thin, the logic gets "
            "RED-first tests against the mock adapter (job reaches Done through the "
            "spawn path; cancel mid-Running lands Canceled in the journal and workers "
            "stop leasing; restart after cancel stays Canceled — journal truth).\n\n"
            "## Acceptance criteria\n"
            "- RED: through the shell's Engine layer (no webview), create_job never "
            "reaches Running today (nothing spawns run_job) — test proves the spawn "
            "path moves a mock job to Done.\n"
            "- RED: `cancel()` doesn't exist — compile-fail/absence proven, then "
            "implementation with journal-pinned tests (Canceled survives reopen).\n"
            "- `job_updated` emitter: test asserts snapshots flow and stop on terminal.\n"
            "- No Canceled→anything transitions; Failed/Done unchanged; all existing "
            "119+ tests stay green; fmt + clippy `-D warnings` on BOTH lanes.\n\n"
            "## Refs\n"
            "docs/02 §8 Sprint-1 item 9, §2.3 (UI never computes truth), ADR-002; "
            "depends on #13 (planner honesty) and #14 (app in gates); blocks #16."
        ),
    },
    {
        "title": "app: Sprint-1 UI slice — jobs table with live progress, badges, per-chunk inspector",
        "labels": ["enhancement"],
        "body": (
            "## Problem\n"
            "`App.tsx` is the Sprint-0 connectivity probe (static table, no progress, "
            "no way to create a job). Spec §8's first deliverable is the jobs table: "
            "file, destination + badge, progress, per-chunk inspector.\n\n"
            "## Design (tight slice — the rest of §8 is explicitly later)\n"
            "- Create-job dialog: file picker (`@tauri-apps/plugin-dialog`, official "
            "plugin), destination form (adapter select from `engine_status`-adjacent "
            "registry listing, endpoint URL, max_conns), size via `stat_file`.\n"
            "- Jobs table: file name, adapter + endpoint host, badge chip "
            "(`Resumable ✅` / `One-shot ⚠️` from `capability_badge` — honest badges "
            "rendered verbatim from the engine, never recomputed), state chip, "
            "confirmed/total progress bar fed by `job_updated` snapshots, live speed "
            "as Δconfirmed/Δt over the poll window (presentation math on journal "
            "numbers, not transfer truth), pause/resume/cancel buttons gated by the "
            "engine's legal transitions.\n"
            "- Per-chunk inspector drawer: `snapshot.chunks` as a table (idx, span, "
            "state, bytes_confirmed, adapter_token) — aria2-style transparency.\n"
            "- All engine data flows through the typed `api.ts` contract; no new "
            "runtime deps beyond the dialog plugin; TypeScript strict; components "
            "kept dumb (props in, events out) so vitest covers the reducer/polling "
            "logic without a webview.\n"
            "- Out of scope (filed later if wanted): speed graph, accounts/OAuth, "
            "i18n, drag-and-drop — spec §8 rows beyond the table+inspector.\n\n"
            "## Acceptance criteria\n"
            "- `npm run build` (tsc strict) green; vitest (or equivalent lightweight "
            "runner) covers: progress-bar math from two snapshots, badge mapping, "
            "button-enable rules mirroring the engine's legal transitions.\n"
            "- The table renders real journal data from the engine through #15's "
            "wiring (verified by the Engine-layer tests, since a webview can't run "
            "in CI) — the UI's only job is to not lie about what the engine says.\n"
            "- Manual smoke in the sandbox is impossible (no display): the honest "
            "verification is build + component tests + the wired engine path; noted "
            "on the issue at close.\n\n"
            "## Refs\n"
            "docs/02 §8 (UI/UX plan), §2.3, ADR-006 amendment; depends on #14 + #15."
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
            "User-Agent": "ium-file-m4",
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
        body = spec["body"] + f"\n\n---\n_Milestone: M4: App shell_"
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
