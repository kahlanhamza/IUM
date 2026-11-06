//! Real-shell WebDriver e2e (issue #35) — the shell ↔ supervisor seam,
//! driven once through the real Tauri app: create → live progress →
//! pause/resume → Done badge → inspector (inspect_chunks through real IPC)
//! → cancel leaves NO server object.
//!
//! **The locator inventory (#68 — the seam checklist; restyle against THIS,
//! not against a red run):**
//! * `#upload-path` / `#upload-endpoint` / `#upload-adapter` /
//!   `#upload-connections` / `#upload-submit` — the intake seams (ids).
//! * adapter `<option>` VALUES are the engine registry names.
//! * rows are located via `table tbody tr:nth-child(i)`; file name lives in
//!   the FIRST td; the FIFTH td's text starts with `NN%`.
//! * every row's state cell carries a `[data-state]` chip (journal truth).
//! * the inspector drawer is found by its ACCESSIBLE ROLE —
//!   `[role="dialog"]` — never by a tag name (the `aside` tag broke once
//!   already when #67 moved the role to its legal host; roles are the
//!   contract, tags are the accident).
//!
//! Leg A runs the **tus adapter** (issue #37 CI debt): per-PATCH server
//! acks make `bytes_confirmed` advance DURING the transfer, so the UI's
//! percent genuinely climbs — with the one-shot family it honestly cannot
//! (one chunk, confirmed only at PUT completion; runs #65–#69 failed on
//! exactly that contradiction). Leg B stays one-shot: the cancel leg pins
//! the honest "nothing committed" semantics of that family.
//!
//! Env contract (set by `scripts/app-e2e.sh`, the same env-gated idiom as
//! the S3/TUS wire suites — without it every test skips LOUDLY):
//! * `IUM_E2E_DRIVER`   — tauri-driver base URL (e.g. http://127.0.0.1:4444)
//! * `IUM_E2E_APP`      — path to the built IUM app binary (tauri-driver
//!   hands it to WebKitWebDriver, which spawns it)
//! * `IUM_E2E_FILE_A` / `IUM_E2E_FILE_B` — source files (A: lifecycle leg
//!   via tus; B: cancel leg via one-shot PUT)
//! * `IUM_E2E_ENDPOINT_A` / `IUM_E2E_ENDPOINT_B` — A: the recorder's tus
//!   creation URL (`.../files`); B: the recorder's PUT URL (`.../upload`)
//! * `IUM_E2E_RECORD_A` / `IUM_E2E_RECORD_B` — recorder record JSON paths
//!   (`{"put_count": n, "received_bytes": n, "tus_sessions": n,
//!   "tus_bytes": n, "tus_last": "<id>"}`, rewritten after every request
//!   by `loopback-recorder`)
//!
//! The WebDriver client is hand-rolled W3C JSON-over-HTTP (the engine
//! hand-rolls sigv4 for the same reason: the protocol is small, the deps
//! stay zero). No websocket endpoints are used — element driving is plain
//! HTTP.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// env-gated entry (the wire-suite idiom)
// ---------------------------------------------------------------------------

struct Env {
    driver: String,
    app: String,
    file_a: String,
    file_b: String,
    endpoint_a: String,
    endpoint_b: String,
    record_a: String,
    record_b: String,
}

fn env_contract() -> Option<Env> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let driver = get("IUM_E2E_DRIVER")?;
    Some(Env {
        app: get("IUM_E2E_APP")?,
        file_a: get("IUM_E2E_FILE_A")?,
        file_b: get("IUM_E2E_FILE_B")?,
        endpoint_a: get("IUM_E2E_ENDPOINT_A")?,
        endpoint_b: get("IUM_E2E_ENDPOINT_B")?,
        record_a: get("IUM_E2E_RECORD_A")?,
        record_b: get("IUM_E2E_RECORD_B")?,
        driver,
    })
}

// ---------------------------------------------------------------------------
// minimal W3C WebDriver client
// ---------------------------------------------------------------------------

struct Wd {
    http: reqwest::Client,
    base: String,
    session: String,
}

impl Wd {
    async fn connect(base: &str, app: &str) -> Result<Wd, String> {
        // NO blanket default timeout: session creation cold-boots the whole
        // app under WebKitWebDriver (first launch on a CI runner is tens of
        // seconds) — a short client timeout aborts the POST mid-handshake,
        // which the server logs as IncompleteMessage and the client sees as
        // "driver not accepting yet". Patient per-request timeout instead.
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| e.to_string())?;
        // tauri-driver's session contract (read from its source, not guessed):
        // `tauri:options.application` is translated into
        // `webkitgtk:browserOptions {binary, args}` for WebKitWebDriver.
        let payload = json!({
            "capabilities": {
                "alwaysMatch": { "tauri:options": { "application": app } }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(240);
        let mut last = String::new();
        while Instant::now() < deadline {
            let resp = http
                .post(format!("{base}/session"))
                .timeout(Duration::from_secs(180))
                .json(&payload)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let v: Value = r.json().await.map_err(|e| e.to_string())?;
                    let sid = v["value"]["sessionId"]
                        .as_str()
                        .ok_or_else(|| format!("no sessionId in {v}"))?
                        .to_string();
                    return Ok(Wd {
                        http,
                        base: base.to_string(),
                        session: sid,
                    });
                }
                Ok(r) => {
                    last = format!(
                        "HTTP {}: {}",
                        r.status(),
                        r.text().await.unwrap_or_default()
                    );
                }
                Err(e) => {
                    last = format!("driver not accepting yet: {}", err_chain(&e));
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(format!("session creation failed, last: {last}"))
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Value,
    ) -> Result<Value, String> {
        let url = format!("{}/session/{}{}", self.base, self.session, path);
        let req = match &body {
            v if v.is_null() => self.http.request(method, &url),
            v => self.http.request(method, &url).json(v),
        };
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            let err = v["value"]["error"].as_str().unwrap_or("unknown");
            let msg = v["value"]["message"].as_str().unwrap_or("");
            return Err(format!("wd {status} {err}: {msg} (POST {path} {body})"));
        }
        Ok(v["value"].clone())
    }

    async fn find(&self, css: &str) -> Result<String, String> {
        let v = self
            .call(
                reqwest::Method::POST,
                "/element",
                json!({"using": "css selector", "value": css}),
            )
            .await?;
        let eid = v[VALUE_ELEMENT_KEY]
            .as_str()
            .ok_or_else(|| format!("no element id in {v}"))?;
        Ok(eid.to_string())
    }

    /// Find with an explicit deadline — the WD protocol has no implicit
    /// wait; every UI wait in this suite is an explicit poll.
    async fn wait_for_element(&self, css: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.find(css).await {
                Ok(eid) => return Ok(eid),
                Err(e) if Instant::now() < deadline => {
                    let _ = e;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(e) => return Err(format!("timeout waiting for `{css}`: {e}")),
            }
        }
    }

    async fn click(&self, eid: &str) -> Result<(), String> {
        self.call(
            reqwest::Method::POST,
            &format!("/element/{eid}/click"),
            json!({}),
        )
        .await
        .map(|_| ())
    }

    async fn send_keys(&self, eid: &str, text: &str) -> Result<(), String> {
        self.call(
            reqwest::Method::POST,
            &format!("/element/{eid}/value"),
            json!({"text": text, "value": text.chars().collect::<Vec<_>>()}),
        )
        .await
        .map(|_| ())
    }

    async fn clear(&self, eid: &str) -> Result<(), String> {
        self.call(
            reqwest::Method::POST,
            &format!("/element/{eid}/clear"),
            json!({}),
        )
        .await
        .map(|_| ())
    }

    async fn text(&self, eid: &str) -> Result<String, String> {
        self.call(
            reqwest::Method::GET,
            &format!("/element/{eid}/text"),
            Value::Null,
        )
        .await?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "text: not a string".into())
    }

    async fn attr(&self, eid: &str, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .call(
                reqwest::Method::GET,
                &format!("/element/{eid}/attribute/{name}"),
                Value::Null,
            )
            .await?
            .as_str()
            .map(str::to_string))
    }

    /// W3C execute-sync — used ONLY to pick an `<option>` out of the
    /// adapter `<select>`: WebKitGTK renders that popup as a native menu
    /// no WebDriver click can reach under xvfb. The script fires the same
    /// React `change` event a user's pick would (native value setter, not
    /// a bare `.value =` — React's value tracker would swallow it); every
    /// other interaction in this suite is real input plumbing.
    async fn set_select(&self, id: &str, value: &str) -> Result<(), String> {
        let script = "const el = document.getElementById(arguments[0]);\
            if (!el) { throw new Error('no #' + arguments[0]); }\
            const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;\
            setter.call(el, arguments[1]);\
            el.dispatchEvent(new Event('change', {bubbles: true}));\
            return el.value;";
        let got = self
            .call(
                reqwest::Method::POST,
                "/execute/sync",
                json!({"script": script, "args": [id, value]}),
            )
            .await?;
        if got.as_str() != Some(value) {
            return Err(format!("select #{id} did not take `{value}`: {got}"));
        }
        Ok(())
    }
}

/// The W3C element key (draft spec identifier WebKit uses).
const VALUE_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// reqwest's Display hides the root cause (a timeout reads as "error
/// sending request"); walk the source chain so forensics land in the log.
fn err_chain(e: &dyn std::error::Error) -> String {
    let mut chain = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        chain.push_str(&format!(" <- {s}"));
        src = s.source();
    }
    chain
}

// ---------------------------------------------------------------------------
// assertions over the UI's journal truth
// ---------------------------------------------------------------------------

/// State of the Nth job row (the `data-state` chip on the state cell).
async fn row_state(wd: &Wd, row_css: &str, timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match wd
            .wait_for_element(&format!("{row_css} [data-state]"), Duration::from_secs(2))
            .await
        {
            Ok(chip) => {
                if let Some(s) = wd.attr(&chip, "data-state").await? {
                    return Ok(s);
                }
            }
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(format!("no [data-state] in {row_css} before deadline"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The first row's rendered progress percent (parsed from the cell text).
async fn row_percent(wd: &Wd, row_css: &str) -> Result<u64, String> {
    let cell = wd
        .wait_for_element(
            &format!("{row_css} td:nth-child(5)"),
            Duration::from_secs(10),
        )
        .await?;
    let t = wd.text(&cell).await?;
    let pct = t
        .split('%')
        .next()
        .and_then(|p| p.trim().parse::<u64>().ok())
        .ok_or_else(|| format!("no percent in cell text '{t}'"))?;
    Ok(pct)
}

/// Resolve a job row BY CONTENT: the first cell holds the source's basename
/// (`JobsTable::fileName`). Positional selectors lie the moment two rows
/// exist — the jobs list renders NEWEST-FIRST (the manager's `list_jobs`
/// order), so run #73's `tr:last-child` was job A's row (done, 100%) while
/// job B ran in row 1; the cancel leg watched the wrong job for 30 s.
async fn row_for_file(wd: &Wd, file_name: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        for i in 1..=16 {
            let cell_css = format!("table tbody tr:nth-child({i}) td:first-child");
            let Ok(cell) = wd.wait_for_element(&cell_css, Duration::from_secs(1)).await else {
                break; // no row i — end of the table
            };
            let t = wd.text(&cell).await.unwrap_or_default();
            if t.contains(file_name) {
                return Ok(format!("table tbody tr:nth-child({i})"));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!("no row whose file cell contains '{file_name}'"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Wait until the row's state chip equals `want` AND, when `pct_below` is
/// set, the rendered percent is strictly below it (lets the test act EARLY
/// in a throttled run — pause/cancel must land mid-transfer, not in the
/// verify gate).
async fn wait_state(
    wd: &Wd,
    row_css: &str,
    want: &str,
    timeout: Duration,
    pct_below: Option<u64>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = row_state(wd, row_css, Duration::from_secs(1))
            .await
            .unwrap_or_default();
        let pct = row_percent(wd, row_css).await.unwrap_or(0);
        let pct_ok = pct_below.map(|cap| pct < cap).unwrap_or(true);
        if state == want && pct_ok {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "row never reached `{want}` (pct cap {pct_below:?}); last state={state} pct={pct}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The recorder's record file — the e2e's server-truth surface (JSON v2:
/// the tus lane's sessions ride alongside the PUT lane's counters).
struct RecordFile {
    received_bytes: u64,
    tus_bytes: u64,
    tus_last: Option<String>,
}

async fn read_record(path: &str) -> Result<RecordFile, String> {
    for _ in 0..40 {
        if let Ok(s) = std::fs::read_to_string(path) {
            let v: Value = serde_json::from_str(&s)
                .map_err(|e| format!("record {path} unparsable: {e} (content: {s})"))?;
            return Ok(RecordFile {
                received_bytes: v["received_bytes"].as_u64().unwrap_or(0),
                tus_bytes: v["tus_bytes"].as_u64().unwrap_or(0),
                tus_last: v["tus_last"].as_str().map(str::to_string),
            });
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!("record file {path} never appeared"))
}

async fn head_status(url: &str) -> Result<u16, String> {
    let http = reqwest::Client::new();
    let resp = http.head(url).send().await.map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

/// HEAD a tus session URL the way the protocol demands (`Tus-Resumable`) —
/// the recorder answers 412 without it. Returns the server-acked offset:
/// for the done leg this is the whole object's size, read from the SERVER.
async fn head_tus_offset(url: &str) -> Result<u64, String> {
    let http = reqwest::Client::new();
    let resp = http
        .head(url)
        .header("Tus-Resumable", "1.0.0")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HEAD {url} -> {}", resp.status()));
    }
    resp.headers()
        .get("upload-offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .ok_or_else(|| format!("HEAD {url}: no upload-offset header"))
}

// ---------------------------------------------------------------------------
// the test — one session, two legs, the surface driven once
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shell_drives_create_progress_pause_resume_done_inspect_and_cancel() {
    let Some(env) = env_contract() else {
        eprintln!(
            "skipped (loud): IUM_E2E_* not set — the real-shell lane runs via \
             scripts/app-e2e.sh (tauri-driver + WebKitWebDriver + loopback-recorder)"
        );
        return;
    };
    for (k, v) in [
        ("file_a", env.file_a.as_str()),
        ("file_b", env.file_b.as_str()),
    ] {
        assert!(Path::new(v).exists(), "e2e source {k} missing: {v}");
    }

    let wd = Wd::connect(&env.driver, &env.app)
        .await
        .expect("webdriver session against the real shell");

    // Self-diagnosing boot dump (the #28 lesson: a RED must say WHY): what
    // page did the webview actually load, and what does its DOM look like?
    let url = wd
        .call(reqwest::Method::GET, "/url", Value::Null)
        .await
        .map(|v| v.as_str().unwrap_or("?").to_string())
        .unwrap_or_else(|e| format!("unavailable: {e}"));
    let title = wd
        .call(reqwest::Method::GET, "/title", Value::Null)
        .await
        .map(|v| v.as_str().unwrap_or("?").to_string())
        .unwrap_or_else(|e| format!("unavailable: {e}"));
    let source = wd
        .call(reqwest::Method::GET, "/source", Value::Null)
        .await
        .map(|v| v.as_str().unwrap_or("").to_string())
        .unwrap_or_else(|e| format!("unavailable: {e}"));
    eprintln!(
        "[e2e] boot dump: url={url} title={title} source_len={} head={:?}",
        source.len(),
        source.chars().take(400).collect::<String>()
    );

    // ---- app booted: the form reflects the engine registry -----------------
    // The Upload button is disabled until `list_adapters` resolved and a path
    // + endpoint exist — waiting for it to become enabled IS the boot sync.
    // 90 s: a cold CI runner's first GTK/WebKit render is slow; a blank or
    // error page is diagnosed by the boot dump above, not guessed.
    let path_input = wd
        .wait_for_element("#upload-path", Duration::from_secs(90))
        .await
        .expect("path seam");
    let endpoint_input = wd
        .wait_for_element("#upload-endpoint", Duration::from_secs(10))
        .await
        .expect("endpoint input");
    let submit = wd
        .wait_for_element("#upload-submit", Duration::from_secs(10))
        .await
        .expect("submit button");

    // ---- leg A: create → live progress → pause → resume → Done -------------
    // The tus adapter is selected EXPLICITLY: the live-progress leg needs a
    // transport whose bytes_confirmed advances during the transfer (runs
    // #65–#69: one-shot plans one chunk and confirms only at PUT completion,
    // so the honest UI percent cannot move and the old two-sample assert was
    // unsatisfiable by construction). Connections go to 6 so the 24 MiB
    // file plans ≥5 chunks (CHUNK_MIN 5 MiB floor respected) — percent
    // steps land every few seconds under the recorder's throttle.
    wd.set_select("upload-adapter", "tus")
        .await
        .expect("select the tus adapter");
    wd.send_keys(&path_input, &env.file_a)
        .await
        .expect("type path A");
    wd.send_keys(&endpoint_input, &env.endpoint_a)
        .await
        .expect("type endpoint A");
    let conns_input = wd
        .wait_for_element("#upload-connections", Duration::from_secs(10))
        .await
        .expect("connections input");
    wd.clear(&conns_input).await.expect("clear connections");
    wd.send_keys(&conns_input, "6")
        .await
        .expect("set 6 connections");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let disabled = wd.attr(&submit, "disabled").await.expect("submit attr");
        if disabled.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "submit never became enabled");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    wd.click(&submit).await.expect("click Upload");

    // The row is matched by FILE (newest-first list — see row_for_file).
    let row_a = row_for_file(&wd, "e2e-a.bin").await.expect("row A by file");
    wait_state(&wd, &row_a, "running", Duration::from_secs(30), None)
        .await
        .expect("job A running");

    // LIVE progress: the percent must CLIMB while the job is still running
    // — polled with a deadline, not two blind samples: under the throttle a
    // chunk boundary lands every few seconds and a 2-core CI runner can
    // delay the first boundary past any fixed pair (the #65–#69 lesson).
    let p0 = row_percent(&wd, &row_a).await.expect("progress sample 0");
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let p = row_percent(&wd, &row_a).await.expect("progress poll");
        if p > p0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "live progress never moved: stuck at {p0}% for 45s"
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    // PAUSE early in a throttled run (mid-transfer, never in the verify gate).
    let pause_btn = wd
        .wait_for_element(
            &format!("{row_a} button[aria-label^='pause']"),
            Duration::from_secs(10),
        )
        .await
        .expect("pause button");
    wd.click(&pause_btn).await.expect("click Pause");
    wait_state(&wd, &row_a, "paused", Duration::from_secs(60), None)
        .await
        .expect("job A paused");

    // RESUME → Done badge.
    let resume_btn = wd
        .wait_for_element(
            &format!("{row_a} button[aria-label^='resume']"),
            Duration::from_secs(10),
        )
        .await
        .expect("resume button");
    wd.click(&resume_btn).await.expect("click Resume");
    wait_state(&wd, &row_a, "done", Duration::from_secs(120), None)
        .await
        .expect("job A done");

    // INSPECTOR = inspect_chunks through real IPC: click the row, read the
    // journal truth the drawer renders verbatim (columns: #, Span, State,
    // Confirmed, Token).
    //
    // Click a CELL of the row, not the <tr>: the jobs table is wider than
    // the window (seven columns, three action buttons), and a full-width
    // row cannot be scrolled into view horizontally — the click atom then
    // answers "element not interactable" (run #72; geckodriver#1294 is the
    // same mechanism). A user clicks a cell anyway: the onClick lives on
    // the <tr> and every cell bubbles to it — the same user path, no seam.
    let row_cell = wd
        .wait_for_element(&format!("{row_a} td:first-child"), Duration::from_secs(10))
        .await
        .expect("row A cell");
    wd.click(&row_cell)
        .await
        .expect("click row A opens inspector");
    wd.wait_for_element("[role=\"dialog\"]", Duration::from_secs(10))
        .await
        .expect("inspector drawer");
    let deadline = Instant::now() + Duration::from_secs(20);
    let (states, confirmed_sum) = loop {
        let mut states = Vec::new();
        let mut confirmed = Vec::new();
        for i in 1..=64 {
            let Some(state_cell) = wd
                .wait_for_element(
                    &format!("[role=\"dialog\"] table tbody tr:nth-child({i}) td:nth-child(3)"),
                    Duration::from_secs(2),
                )
                .await
                .ok()
            else {
                break; // no more chunk rows
            };
            states.push(wd.text(&state_cell).await.unwrap_or_default());
            if let Ok(conf_cell) = wd
                .wait_for_element(
                    &format!("[role=\"dialog\"] table tbody tr:nth-child({i}) td:nth-child(4)"),
                    Duration::from_secs(2),
                )
                .await
            {
                let t = wd.text(&conf_cell).await.unwrap_or_default();
                confirmed.push(t.trim().parse::<u64>().unwrap_or(0));
            }
        }
        if !states.is_empty()
            && states.len() == confirmed.len()
            && states.iter().all(|s| s == "uploaded" || s == "verified")
        {
            break (states, confirmed.iter().sum::<u64>());
        }
        assert!(
            Instant::now() < deadline,
            "chunks never all-confirmed: {states:?} {confirmed:?}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    assert!(!states.is_empty(), "inspector rendered no chunk rows");
    assert_eq!(
        confirmed_sum,
        std::fs::metadata(&env.file_a).unwrap().len(),
        "Σ bytes_confirmed across the plan == the source size (journal truth)"
    );

    // Close the drawer BEFORE leg B: it is a fixed-position overlay on the
    // window's right edge — the next row's action buttons live underneath
    // it, and a WebDriver click lands on the overlay, not the button.
    let close_btn = wd
        .wait_for_element("[role=\"dialog\"] button", Duration::from_secs(5))
        .await
        .expect("inspector close button");
    wd.click(&close_btn).await.expect("close inspector");

    // Server truth: the recorder's tus session holds every byte, and HEAD
    // on the REAL session URL (the record file surfaces its id) acks the
    // full offset — done per the SERVER, not the badge.
    let rec_a = read_record(&env.record_a).await.expect("record A");
    let source_a = std::fs::metadata(&env.file_a).unwrap().len();
    assert_eq!(
        rec_a.tus_bytes, source_a,
        "recorder's tus session holds every source byte"
    );
    let session_url = format!(
        "{}/{}",
        env.endpoint_a.trim_end_matches('/'),
        rec_a.tus_last.expect("recorder created a tus session")
    );
    let offset = head_tus_offset(&session_url)
        .await
        .expect("HEAD tus session");
    assert_eq!(
        offset, source_a,
        "HEAD upload-offset == source size on the recorder's session"
    );

    // ---- leg B: cancel mid-transfer → Canceled badge → NO server object ----
    // Back to the one-shot adapter explicitly: leg A left the select on tus,
    // and the cancel leg pins the one-shot family's honest "nothing
    // committed" ending.
    wd.set_select("upload-adapter", "http_oneshot")
        .await
        .expect("select the one-shot adapter back");
    let path_input = wd
        .wait_for_element("#upload-path", Duration::from_secs(10))
        .await
        .expect("path seam again");
    let endpoint_input = wd
        .wait_for_element("#upload-endpoint", Duration::from_secs(10))
        .await
        .expect("endpoint input again");
    wd.clear(&path_input).await.ok();
    wd.send_keys(&path_input, &env.file_b)
        .await
        .expect("type path B");
    wd.clear(&endpoint_input).await.ok();
    wd.send_keys(&endpoint_input, &env.endpoint_b)
        .await
        .expect("type endpoint B");
    let submit = wd
        .wait_for_element("#upload-submit", Duration::from_secs(10))
        .await
        .expect("submit again");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let disabled = wd.attr(&submit, "disabled").await.expect("submit attr");
        if disabled.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "submit B never became enabled");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    wd.click(&submit).await.expect("click Upload B");

    // By FILE again: after B's creation the list re-orders to [B, A] —
    // run #73 watched job A's row here (`last state=done pct=100`) while
    // the throttled one-shot PUT ran to completion in row 1.
    let row_b = row_for_file(&wd, "e2e-b.bin").await.expect("row B by file");
    wait_state(&wd, &row_b, "running", Duration::from_secs(30), None)
        .await
        .expect("job B running");

    // Cancel EARLY — the throttled recorder makes every PUT take seconds, so
    // the drop lands mid-body: the recorder must commit NOTHING.
    let cancel_btn = wd
        .wait_for_element(
            &format!("{row_b} button[aria-label^='cancel']"),
            Duration::from_secs(10),
        )
        .await
        .expect("cancel button");
    wd.click(&cancel_btn).await.expect("click Cancel");
    wait_state(&wd, &row_b, "canceled", Duration::from_secs(30), None)
        .await
        .expect("job B canceled");

    tokio::time::sleep(Duration::from_millis(500)).await; // let the recorder settle
    let rec_b = read_record(&env.record_b).await.expect("record B");
    assert_eq!(
        rec_b.received_bytes, 0,
        "a canceled upload never completed the object — recorder holds nothing"
    );
    let head = head_status(&env.endpoint_b).await.expect("HEAD endpoint B");
    assert_eq!(head, 404, "canceled upload → HEAD 404 (no server object)");

    // Clean shutdown of the session (and the app it spawned).
    let http = reqwest::Client::new();
    let _ = http
        .delete(format!("{}/session/{}", env.driver, wd.session))
        .send()
        .await;
}
