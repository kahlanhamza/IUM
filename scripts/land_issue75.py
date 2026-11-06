"""Post the landing evidence comment on #75."""
import json
import urllib.request
from pathlib import Path

TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()
H = {
    "Authorization": f"Bearer {TOKEN}",
    "Accept": "application/vnd.github+json",
    "User-Agent": "ium-loop",
}

body = """## LANDED — the telemetry stance shipped and CI-green (lane 7 of #66 complete)

**The loop, on the record:**

- **Owner decision** (this session, quoted verbatim in the design note): option (a) — full telemetry consent model, opt-out, the big-tech standard, users trust first.
- **Wire facts W1–W7** read from the tree before any design: the journal's plain-text `source_path`/`endpoint` must never reach an event (allowlist built from fields, never from records); the watcher's terminal transition + `setup` are the observation points; the vault pattern (0600, atomic, corrupt=default, standalone harness); the brand-gate drift pattern reused for the charter; **no sink endpoint exists** — the sink is the pinned owner ask (the lane-4 signing precedent).
- **Issue filed BEFORE implementation** (design note = the body). **RED captured at HEAD `c1e926e`** (comment 6694066013): charter ENOENT on both sides — Rust `rustc --test` gate-only form FAILED on the missing charter; vitest 4/4 failed on the missing mirror.
- **Landed in `ebc1660`** (auto-closed this issue on the Closes-keyword):
  - `docs/11_Telemetry_Stance.md` — the WRITTEN stance (the #66 acceptance bar): the decision, the 11-field collected-fields table, the never-collected list, where it lands, the switches, the honest limits (verbatim panic message), the gates, how the stance may change.
  - `ium-app/src-tauri/src/telemetry.rs` — pure-std (no serde/tracing/tauri; hand-rolled, escape-tested JSON): consent opt-out enforced at THE single write point (off = zero file IO), 0600 atomic consent (corrupt = default, live switch), capped 500 / rotate 250 local JSONL buffer, clear + status.
  - Shell wiring: `OnceLock` + chained panic hook (the real panic still prints) + the watcher's terminal event + `telemetry_status` / `telemetry_set_consent` / `telemetry_clear`.
  - Product surface: **Settings → Privacy leads the dialog** — stance sentence, master switch, what-we-collect sentences (`TELEMETRY_COPY`, voice-gated via the brand gate's enumerated surface), buffer count + absolute path + two-step clear.
  - **The charter cannot drift**: the Rust harness parses the charter's table and requires per-kind equality with the code constants; the vitest charter gate compares the same table to `TELEMETRY_FIELDS`. Neither side moves without the other.
  - **NO transport ships in this build** — no HTTP client in the shell, no endpoint: nothing leaves the device BY CONSTRUCTION. The sink (Sentry DSN or self-hosted) is the pinned owner ask; when it ships it gates on consent AND endpoint AND a one-time disclosure.

**The gates earned their keep — 4 real defects caught pre-ship:**

1. The standalone Rust harness caught `str::lines()` stripping newlines — the rewrite-after-read GLUED every record into one line (`split_inclusive` + unterminated-tail normalize fixed it).
2. The same harness caught a wrong `starts_with` prefix in my own test (the closing brace).
3. It caught the cap test asserting phase arithmetic instead of the invariant (stored ≤ 500, rotation fired, newest rides).
4. **CI caught what the Linux sandbox could not compile**: clippy `-D warnings` failed `ium-app` on all three OSes — the allowlist constants were only used by the drift test while the writers hand-rolled the field names. "Allowlist-by-construction" was not literally true. Fixed in `0d0be45`: `emit()` assembles every event FROM the constants (length-asserted lineup, charter row order). A field can now appear in an event only by being in a constant the drift test pins to the charter.

**Gates:** standalone harness **11/11**; non-test build warning-free; vitest **60/60** (56 + 4 charter); tsc strict; vite build 189.71 KB JS; fmt clean; gate.sh GREEN on the committed tree; **CI run 42290227532 on `0d0be45`: 8/8 GREEN, all three OSes** incl. app-e2e.

Product-bar lanes 1–7 complete (#67/#69/#70/#71/#72/#73/#75); remaining: lane 8 (RE-resistance checklist at release)."""
req = urllib.request.Request(
    "https://api.github.com/repos/kahlanhamza/IUM/issues/75/comments",
    data=json.dumps({"body": body}).encode(),
    headers={**H, "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=30) as r:
    c = json.loads(r.read().decode())
print("landing comment posted:", c["id"])
