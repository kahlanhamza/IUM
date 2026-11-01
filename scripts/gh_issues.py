#!/usr/bin/env python3
"""Idempotent GitHub milestone/label/issue bootstrap for IUM.

Reads the PAT from /home/z/my-project/.secrets/gh_token (never printed, never logged).
Safe to re-run: existing labels/milestones/issues are detected by name/title and skipped.

Usage: python3 scripts/gh_issues.py
"""
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN_PATH = Path("/home/z/my-project/.secrets/gh_token")

LABELS = [
    ("core", "d4c5f9", "ium-core engine"),
    ("tests", "0e8a16", "test suite work"),
    ("ci", "e99695", "continuous integration / tooling"),
    ("process", "fbca04", "engineering process & docs"),
]

MILESTONE = {
    "title": "M2: Hardening & Honesty",
    "description": "docs/04_Roadmap.md M2 — verify cannot be opt-in, loud-failure symmetry, "
    "nightly fuzz, mock observability. DoD: gates green on 3 OS, failing-test-first per fix.",
}

ISSUES = [
    {
        "title": "adapter: make TransferAdapter::verify a required trait method",
        "labels": ["core", "enhancement"],
        "body": (
            "## Problem\n`TransferAdapter::verify` has a default impl that returns a passing "
            "`VerifyReport` (src/adapter.rs). A future adapter that forgets to override it "
            "silently inherits verify-before-declare as a no-op — the job reaches Done on the "
            "strength of a default pass.\n\n"
            "## Acceptance criteria\n"
            "- `verify` is a required trait method (no default impl); compilation fails if an "
            "adapter does not implement it\n"
            "- existing adapters (HttpOneShotAdapter, test mocks) implement it explicitly\n"
            "- full gates green: fmt + clippy -D warnings + cargo test -p ium-core\n\n"
            "## Refs\nADR-004 (verify-before-declare), docs/04_Roadmap.md M2"
        ),
    },
    {
        "title": "journal: rows-affected check on DELETE in replace_chunks",
        "labels": ["core", "bug"],
        "body": (
            "## Problem\n`replace_chunks` (src/journal.rs) pre-checks job existence but the bare "
            "`DELETE FROM chunks` does not verify how many rows it removed — the only write path "
            "without a loud-failure check, breaking the symmetry established by "
            "`set_job_state`/`update_chunk`.\n\n"
            "## Acceptance criteria\n"
            "- DELETE asserts expected rows affected (or documents why a count of 0 is legal)\n"
            "- failing test first: a replace on a job whose chunks were already removed must "
            "error loudly, not silently no-op\n\n"
            "## Refs\nADR-002 (journal is truth; writes fail loudly), Sprint 0 audit item #2"
        ),
    },
    {
        "title": "tests: MockAdapter should log calls at entry (observability parity)",
        "labels": ["tests"],
        "body": (
            "## Problem\nMockAdapter (tests/integration.rs) pushes its UploadLog only when a "
            "call completes. Failing/evicted calls are therefore invisible — the same gap that "
            "made `stalled_chunk_is_evicted_and_job_still_completes` unassertable until the "
            "ScriptedAdapter fix (log at entry, patch bytes_moved at completion).\n\n"
            "## Acceptance criteria\n"
            "- MockAdapter logs at entry; bytes_moved patched at completion\n"
            "- no existing assertion breaks; gates green\n\n"
            "## Refs"
            " 2026-11-02 gate fixes; docs/05_Engineering_Process.md (postmortem rule)"
        ),
    },
    {
        "title": "ci: nightly chunk-pool fuzz run + coverage report",
        "labels": ["ci", "tests"],
        "body": (
            "## Problem\nThe chunk-pool invariant fuzz (`fuzz_random_operations_never_break_"
            "invariants`, src/chunk.rs) runs 25 rounds x 400 ops in the normal suite. Cheap "
            "in CI, but shallow. A scheduled expanded run costs nothing in PR latency and "
            "catches rare lease/partial-confirm interleavings.\n\n"
            "## Acceptance criteria\n"
            "- scheduled (nightly) workflow: fuzz with expanded rounds (env-gated), ubuntu only\n"
            "- coverage report artifact for ium-core (e.g. cargo-llvm-cov), uploaded per run\n\n"
            "## Refs\ndocs/04_Roadmap.md M2"
        ),
    },
]


def api(method: str, path: str, payload: dict | None = None):
    url = f"https://api.github.com/repos/{REPO}/{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {TOKEN_PATH.read_text().strip()}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "ium-bootstrap",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            body = resp.read()
            return resp.status, (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:400]


def main() -> int:
    if not TOKEN_PATH.exists() or not TOKEN_PATH.read_text().strip():
        print(
            "BLOCKED: token missing. Paste the PAT into .secrets/gh_token "
            "(rotate it first — it appeared in chat once):\n"
            "  mkdir -p .secrets && chmod 700 .secrets\n"
            "  echo 'TOKEN' > .secrets/gh_token && chmod 600 .secrets/gh_token"
        )
        return 2

    for name, color, desc in LABELS:
        status, _ = api("POST", "labels", {"name": name, "color": color, "description": desc})
        print(f"label {name}: {'created' if status == 201 else f'exists/http-{status}'}")

    status, milestones = api("GET", "milestones?state=all")
    milestone_number = None
    if status == 200 and milestones:
        milestone_number = next(
            (m["number"] for m in milestones if m["title"] == MILESTONE["title"]), None
        )
    if milestone_number is None:
        status, m = api("POST", "milestones", MILESTONE)
        milestone_number = m["number"] if status == 201 else None
    print(f"milestone: #{milestone_number} ({MILESTONE['title']})")

    status, existing = api("GET", "issues?state=all&per_page=100")
    existing_titles = {i["title"]: i["number"] for i in existing} if status == 200 else {}

    for issue in ISSUES:
        if issue["title"] in existing_titles:
            print(f"issue: EXISTS #{existing_titles[issue['title']]} — {issue['title']}")
            continue
        payload = {"title": issue["title"], "body": issue["body"], "labels": issue["labels"]}
        if milestone_number:
            payload["milestone"] = milestone_number
        status, created = api("POST", "issues", payload)
        if status == 201:
            print(f"issue: CREATED #{created['number']} — {issue['title']}")
        else:
            print(f"issue: FAILED http-{status} — {issue['title']}: {created}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
