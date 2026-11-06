//! The telemetry stance made code (#66 Lane 7, #75) — pure-std (no serde,
//! no tauri, no tracing): the sandbox has no WebKitGTK, so this module is
//! proven LOCALLY via `CARGO_MANIFEST_DIR=… rustc --test telemetry.rs` while
//! the crate that embeds it takes its compile proof from CI (the #61/#69
//! standing lesson).
//!
//! The contract is the charter (`docs/11_Telemetry_Stance.md`): events carry
//! ONLY the charter's allowlisted fields (a test below parses the charter and
//! pins it — the stance cannot drift), consent is opt-out and checked at the
//! SINGLE write point, and the buffer is a local, capped, user-clearable
//! JSONL file. NO transport ships in this build — the sink is the pinned
//! owner ask — so nothing leaves the device by construction, not by promise.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const TELEMETRY_FILE: &str = "telemetry.jsonl";
pub const CONSENT_FILE: &str = "telemetry_consent.json";
/// The buffer never grows past CAP records; rotation keeps the newest KEEP.
pub const CAP: usize = 500;
pub const KEEP: usize = 250;

/// Fields every event carries (charter § What IUM collects).
pub const EVERY: &[&str] = &["v", "kind", "ts"];
/// `job_terminal` carries ONLY these beyond `EVERY` — no path, no endpoint,
/// no job id (the journal's plain-text fields must never ride along).
pub const JOB_KEYS: &[&str] = &["adapter", "state", "size_bucket", "duration_bucket"];
/// `panic` carries ONLY these beyond `EVERY` — the message is verbatim (the
/// documented trade, charter § The honest limits); `source` is an IUM source
/// path from the panic Location, never a user path.
pub const PANIC_KEYS: &[&str] = &["message", "source", "line", "column"];

/// The telemetry pipeline. Consent lives here because the OFF decision must
/// sit at the write point, not in a UI layer that can be bypassed.
pub struct Telemetry {
    dir: PathBuf,
    consent: AtomicBool,
}

impl Telemetry {
    /// Opens the pipeline for a directory that already exists. A missing or
    /// corrupt consent file falls back to the DEFAULT (opt-out, collected) —
    /// never a startup panic (the vault's corrupted-is-default honesty).
    pub fn open(dir: &Path) -> Self {
        let t = Self {
            dir: dir.to_path_buf(),
            consent: AtomicBool::new(true),
        };
        if let Ok(s) = std::fs::read_to_string(t.dir.join(CONSENT_FILE)) {
            if let Some(on) = parse_consent(&s) {
                t.consent.store(on, Ordering::Relaxed);
            }
        }
        t
    }

    pub fn consented(&self) -> bool {
        self.consent.load(Ordering::Relaxed)
    }

    /// The only writer of consent: 0600, atomic, immediate (no restart
    /// needed — unlike credentials, this switch must be trusted live).
    pub fn set_consent(&self, on: bool) -> Result<(), String> {
        self.consent.store(on, Ordering::Relaxed);
        let path = self.dir.join(CONSENT_FILE);
        atomic_write(&path, format!("{{\"consent\":{}}}\n", on))?;
        set_0600(&path);
        Ok(())
    }

    /// One event per terminal job — the watcher's final snapshot, non-user
    /// fields only. Wide buckets carry the size and the duration. The values
    /// line up IN ORDER with JOB_KEYS — emit() is the allowlist-by-
    /// construction writer (CI held this claim honest: the first cut
    /// hand-rolled the field names and clippy -D warnings killed the
    /// constants as dead code on all three OSes).
    pub fn record_job_terminal(&self, adapter: &str, state: &str, size: u64, duration_ms: i64) {
        let vals = vec![
            json(adapter),
            json(state),
            json(size_bucket(size)),
            json(duration_bucket(duration_ms)),
        ];
        self.record(emit("job_terminal", JOB_KEYS, vals));
    }

    /// One event per panic — verbatim payload (charter § The honest limits),
    /// the panic's own source location, chained hooks keep the real print.
    pub fn record_panic(&self, message: &str, source: &str, line: u32, column: u32) {
        let vals = vec![
            json(message),
            json(source),
            line.to_string(),
            column.to_string(),
        ];
        self.record(emit("panic", PANIC_KEYS, vals));
    }

    /// THE write point (D3): consent checked here means the switch cannot be
    /// bypassed by any future caller. Best-effort: telemetry must never be
    /// the reason an upload or a crash report fails.
    fn record(&self, line: String) {
        if !self.consented() {
            return;
        }
        let path = self.buf_path();
        let mut lines = read_lines(&path);
        lines.push(line);
        if lines.len() > CAP {
            lines = lines.split_off(lines.len() - KEEP);
        }
        let _ = atomic_write(&path, lines.concat());
    }

    /// (consent, buffered-record count).
    pub fn status(&self) -> (bool, usize) {
        (self.consented(), read_lines(&self.buf_path()).len())
    }

    /// The absolute buffer path — Settings shows it so the file is one
    /// click away (charter § Where it lands).
    pub fn buf_path(&self) -> PathBuf {
        self.dir.join(TELEMETRY_FILE)
    }

    /// Empties the buffer (Settings → Privacy, two-step confirm).
    pub fn clear(&self) -> Result<(), String> {
        atomic_write(&self.buf_path(), String::new())
    }
}

// ---------------------------------------------------------------------------
// Buckets — the width is the anonymity (charter § What IUM collects).
// ---------------------------------------------------------------------------

pub fn size_bucket(size: u64) -> &'static str {
    const MIB: u64 = 1024 * 1024;
    if size <= MIB {
        "lt_1mib"
    } else if size <= 16 * MIB {
        "1_16mib"
    } else if size <= 128 * MIB {
        "16_128mib"
    } else if size <= 1024 * MIB {
        "128mib_1gib"
    } else {
        "gt_1gib"
    }
}

pub fn duration_bucket(ms: i64) -> &'static str {
    const S: i64 = 1000;
    if ms <= S {
        "lt_1s"
    } else if ms <= 10 * S {
        "1_10s"
    } else if ms <= 60 * S {
        "10_60s"
    } else if ms <= 10 * 60 * S {
        "1_10m"
    } else if ms <= 60 * 60 * S {
        "10_60m"
    } else {
        "gt_1h"
    }
}

// ---------------------------------------------------------------------------
// JSON — hand-rolled for FIVE fixed shapes: the pure-std contract that keeps
// the standalone harness honest. The escaper is tested against quotes,
// backslashes, newlines and control bytes (panic payloads are hostile).
// ---------------------------------------------------------------------------

/// THE allowlist-by-construction writer: the object is assembled FROM the
/// charter-gated constants — the EVERY fields first, then the kind's own
/// keys paired with the caller's values IN ORDER. A field can appear in an
/// event only by being in a constant the drift test pins to the charter;
/// the writers cannot grow a field without growing the charter first.
fn emit(kind: &str, kind_keys: &[&str], vals: Vec<String>) -> String {
    assert_eq!(
        kind_keys.len(),
        vals.len(),
        "writer values must line up with the allowlist"
    );
    let every_vals = ["1".to_string(), json(kind), now_ms().to_string()];
    let mut out = String::with_capacity(160);
    out.push('{');
    for (k, v) in EVERY.iter().zip(every_vals.iter()) {
        out.push_str(&format!("\"{k}\":{v},"));
    }
    for (k, v) in kind_keys.iter().zip(vals.iter()) {
        out.push_str(&format!("\"{k}\":{v},"));
    }
    out.pop(); // the trailing comma
    out.push_str("}\n");
    out
}

fn json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn parse_consent(s: &str) -> Option<bool> {
    let key = "\"consent\"";
    let i = s.find(key)? + key.len();
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Reads the buffer PRESERVING each line's newline — `str::lines()` strips
/// it, and a rewrite-after-read would glue every record into one line (the
/// harness caught exactly that pre-commit). A hand-edited unterminated tail
/// is normalized so the next record can never glue to it.
fn read_lines(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let mut out: Vec<String> = s.split_inclusive('\n').map(str::to_string).collect();
            if let Some(last) = out.last_mut() {
                if !last.ends_with('\n') {
                    last.push('\n');
                }
            }
            out
        }
        Err(_) => Vec::new(),
    }
}

fn atomic_write(path: &Path, body: String) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(unix)]
fn set_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_0600(_path: &Path) {}

// ---------------------------------------------------------------------------
// Tests — the charter drift gate first, then every behavioral contract.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn tmpdir(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ium-telemetry-test-{}-{}-{}",
            tag,
            N.fetch_add(1, Ordering::Relaxed),
            now_ms()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn charter_table(doc: &str) -> std::collections::BTreeMap<String, Vec<String>> {
        // rows: | `field` | event | `example` | — the header row has no
        // backticks and dies here (the #73 CRLF lesson: anchor tightly).
        let mut out = std::collections::BTreeMap::new();
        for line in doc.lines() {
            let cols: Vec<&str> = line.split('|').collect();
            if cols.len() < 4 {
                continue;
            }
            let raw = cols[1].trim();
            if !(raw.starts_with('`') && raw.ends_with('`') && raw.len() >= 2) {
                continue;
            }
            let field = &raw[1..raw.len() - 1];
            if !field
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
            {
                continue;
            }
            out.entry(cols[2].trim().to_string())
                .or_insert_with(Vec::new)
                .push(field.to_string());
        }
        out
    }

    #[test]
    fn charter_table_equals_code_allowlist() {
        let doc = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/11_Telemetry_Stance.md"
        ))
        .expect("the charter exists");
        let table = charter_table(&doc);
        let every: Vec<String> = EVERY.iter().map(|s| s.to_string()).collect();
        let job: Vec<String> = JOB_KEYS.iter().map(|s| s.to_string()).collect();
        let panic: Vec<String> = PANIC_KEYS.iter().map(|s| s.to_string()).collect();

        // Each charter group names exactly its own constant — the composed
        // event sets are EVERY ∪ (kind keys), which the writers implement;
        // the groups must not repeat or cross the kinds.
        assert_eq!(
            table.get("every").expect("charter has an `every` group"),
            &every,
            "the charter's every-event fields drifted from the code"
        );
        assert_eq!(
            table
                .get("job_terminal")
                .expect("charter has a job_terminal group"),
            &job,
            "the charter's job_terminal fields drifted from the code"
        );
        assert_eq!(
            table.get("panic").expect("charter has a panic group"),
            &panic,
            "the charter's panic fields drifted from the code"
        );
        // The kind groups must not smuggle the every-fields (the writer
        // emits every ∪ kind — a repeat in the table would hide a drift).
        for (a, b) in [
            ("job_terminal", "every"),
            ("panic", "every"),
            ("job_terminal", "panic"),
        ] {
            for f in table.get(a).unwrap() {
                assert!(
                    !table.get(b).unwrap().contains(f),
                    "{a} repeats a {b} field: {f}"
                );
            }
        }
        for group in table.keys() {
            assert!(
                matches!(group.as_str(), "every" | "job_terminal" | "panic"),
                "unexpected charter field group: {group}"
            );
        }
    }

    #[test]
    fn default_consent_is_opt_out() {
        let dir = tmpdir("default");
        assert!(Telemetry::open(&dir).consented());
    }

    #[test]
    fn consent_off_means_zero_file_io() {
        let dir = tmpdir("off");
        let t = Telemetry::open(&dir);
        t.set_consent(false).unwrap();
        t.record_job_terminal("tus", "done", 100, 1000);
        t.record_panic("boom", "x.rs", 1, 2);
        assert!(!dir.join(TELEMETRY_FILE).exists(), "no file may be written");
        assert_eq!(t.status(), (false, 0));
    }

    #[test]
    fn consent_on_appends_allowlisted_fields_only() {
        let dir = tmpdir("on");
        let t = Telemetry::open(&dir);
        t.record_job_terminal("tus", "done", 2 * 1024 * 1024, 5_000);
        let line = std::fs::read_to_string(dir.join(TELEMETRY_FILE)).unwrap();
        let line = line.trim_end();
        // emit() writes the fields in the CHARTER'S ROW ORDER — the object
        // literally reads top-to-bottom like the stance document.
        assert!(
            line.starts_with("{\"v\":1,\"kind\":\"job_terminal\",\"ts\":"),
            "{line}"
        );
        assert!(line.contains("\"adapter\":\"tus\""), "{line}");
        assert!(line.contains("\"state\":\"done\""), "{line}");
        assert!(line.contains("\"size_bucket\":\"1_16mib\""), "{line}");
        assert!(line.contains("\"duration_bucket\":\"1_10s\""), "{line}");
        // The never-collected list is structurally absent (W2/D2):
        assert!(!line.contains("path"), "{line}");
        assert!(!line.contains("endpoint"), "{line}");
        assert!(!line.contains("job_id"), "{line}");
        assert!(!line.contains("url"), "{line}");
    }

    #[test]
    fn json_escaping_survives_hostile_payloads() {
        let dir = tmpdir("escape");
        let t = Telemetry::open(&dir);
        t.record_panic("a \"b\" \\ c\nd\te\u{7}f", "src/x.rs", 3, 4);
        let line = std::fs::read_to_string(dir.join(TELEMETRY_FILE)).unwrap();
        let line = line.trim_end();
        assert!(
            line.contains("\"message\":\"a \\\"b\\\" \\\\ c\\nd\\te\\u0007f\""),
            "{line}"
        );
        assert!(line.contains("\"source\":\"src/x.rs\""), "{line}");
        assert!(line.contains("\"line\":3"), "{line}");
        assert!(line.contains("\"column\":4"), "{line}");
    }

    #[test]
    fn buckets_are_wide_and_exhaustive() {
        const MIB: u64 = 1024 * 1024;
        assert_eq!(size_bucket(0), "lt_1mib");
        assert_eq!(size_bucket(MIB), "lt_1mib");
        assert_eq!(size_bucket(MIB + 1), "1_16mib");
        assert_eq!(size_bucket(100 * MIB), "16_128mib");
        assert_eq!(size_bucket(500 * MIB), "128mib_1gib");
        assert_eq!(size_bucket(2 * 1024 * MIB), "gt_1gib");
        assert_eq!(duration_bucket(0), "lt_1s");
        assert_eq!(duration_bucket(1_000), "lt_1s");
        assert_eq!(duration_bucket(5_000), "1_10s");
        assert_eq!(duration_bucket(30_000), "10_60s");
        assert_eq!(duration_bucket(5 * 60_000), "1_10m");
        assert_eq!(duration_bucket(30 * 60_000), "10_60m");
        assert_eq!(duration_bucket(2 * 3_600_000), "gt_1h");
    }

    #[test]
    fn buffer_caps_at_500_and_keeps_the_newest_250() {
        let dir = tmpdir("cap");
        let t = Telemetry::open(&dir);
        for i in 0..600 {
            t.record_job_terminal(&format!("a{i}"), "done", 1, 1);
        }
        let lines = read_lines(&t.buf_path());
        // The charter's contract is the INVARIANT, not a phase of the
        // arithmetic: never more than CAP stored, rotation actually fired,
        // the newest record always rides, and the rotation point is exact
        // (record 501 trips the cap; the newest 250 of a0..a500 = a251..a500).
        assert!(
            lines.len() <= CAP,
            "never more than the charter's cap: {}",
            lines.len()
        );
        assert!(lines.len() >= KEEP, "rotation must have fired");
        assert!(
            lines.first().unwrap().contains("\"a251\""),
            "first kept: {}",
            lines[0]
        );
        assert!(
            lines.last().unwrap().contains("\"a599\""),
            "the last record rides"
        );
    }

    #[test]
    fn clear_empties_the_buffer() {
        let dir = tmpdir("clear");
        let t = Telemetry::open(&dir);
        t.record_job_terminal("tus", "done", 1, 1);
        t.record_job_terminal("tus", "failed", 1, 1);
        assert_eq!(t.status().1, 2);
        t.clear().unwrap();
        assert_eq!(t.status(), (true, 0));
    }

    #[test]
    fn corrupt_consent_file_falls_back_to_default() {
        let dir = tmpdir("corrupt");
        std::fs::write(dir.join(CONSENT_FILE), "{not json at all").unwrap();
        assert!(Telemetry::open(&dir).consented());
    }

    #[test]
    fn consent_choice_persists_across_open() {
        let dir = tmpdir("persist");
        Telemetry::open(&dir).set_consent(false).unwrap();
        assert!(!Telemetry::open(&dir).consented());
        Telemetry::open(&dir).set_consent(true).unwrap();
        assert!(Telemetry::open(&dir).consented());
    }

    #[test]
    fn consent_file_is_0600_and_no_tmp_leftovers() {
        let dir = tmpdir("perms");
        let t = Telemetry::open(&dir);
        t.set_consent(false).unwrap();
        t.record_job_terminal("tus", "done", 1, 1); // off — nothing writes
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(CONSENT_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
