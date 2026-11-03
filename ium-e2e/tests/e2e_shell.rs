//! Real-shell WebDriver e2e (issue #35) — the shell ↔ supervisor seam,
//! driven once through the real Tauri app: create → live progress →
//! pause/resume → Done badge → inspector (inspect_chunks through real IPC)
//! → cancel leaves NO server object.
//!
//! Env contract (set by `scripts/app-e2e.sh`, the same env-gated idiom as
//! the S3/TUS wire suites — without it every test skips LOUDLY):
//! * `IUM_E2E_DRIVER`   — tauri-driver base URL (e.g. http://127.0.0.1:4444)
//! * `IUM_E2E_APP`      — path to the built IUM app binary (tauri-driver
//!   hands it to WebKitWebDriver, which spawns it)
//! * `IUM_E2E_FILE_A` / `IUM_E2E_FILE_B` — source files (A: lifecycle leg,
//!   B: cancel leg)
//! * `IUM_E2E_ENDPOINT_A` / `IUM_E2E_ENDPOINT_B` — recorder PUT URLs
//! * `IUM_E2E_RECORD_A` / `IUM_E2E_RECORD_B` — recorder record JSON paths
//!   (`{"put_count": n, "received_bytes": n}`, rewritten after every
//!   request by `loopback-recorder`)
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
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
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
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut last = String::new();
        while Instant::now() < deadline {
            let resp = http
                .post(format!("{base}/session"))
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
                    last = format!("driver not accepting yet: {e}");
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
}

/// The W3C element key (draft spec identifier WebKit uses).
const VALUE_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

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

async fn read_record(path: &str) -> Result<(u64, u64), String> {
    for _ in 0..40 {
        if let Ok(s) = std::fs::read_to_string(path) {
            let v: Value = serde_json::from_str(&s)
                .map_err(|e| format!("record {path} unparsable: {e} (content: {s})"))?;
            return Ok((
                v["put_count"].as_u64().unwrap_or(0),
                v["received_bytes"].as_u64().unwrap_or(0),
            ));
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

    // ---- app booted: the form reflects the engine registry -----------------
    // The Upload button is disabled until `list_adapters` resolved and a path
    // + endpoint exist — waiting for it to become enabled IS the boot sync.
    let path_input = wd
        .wait_for_element("#upload-path", Duration::from_secs(30))
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
    wd.send_keys(&path_input, &env.file_a)
        .await
        .expect("type path A");
    wd.send_keys(&endpoint_input, &env.endpoint_a)
        .await
        .expect("type endpoint A");
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

    let row_a = "table tbody tr:first-child";
    wait_state(&wd, row_a, "running", Duration::from_secs(30), None)
        .await
        .expect("job A running");

    // LIVE progress: two samples while running — the second strictly greater
    // (the throttled recorder makes the 400 ms job_updated ticks visible).
    let p1 = row_percent(&wd, row_a).await.expect("progress sample 1");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let p2 = row_percent(&wd, row_a).await.expect("progress sample 2");
    assert!(p2 > p1, "live progress must move: first={p1}% second={p2}%");

    // PAUSE early in a throttled run (mid-transfer, never in the verify gate).
    let pause_btn = wd
        .wait_for_element(
            &format!("{row_a} button[aria-label^='pause']"),
            Duration::from_secs(10),
        )
        .await
        .expect("pause button");
    wd.click(&pause_btn).await.expect("click Pause");
    wait_state(&wd, row_a, "paused", Duration::from_secs(60), None)
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
    wait_state(&wd, row_a, "done", Duration::from_secs(120), None)
        .await
        .expect("job A done");

    // INSPECTOR = inspect_chunks through real IPC: click the row, read the
    // journal truth the drawer renders verbatim (columns: #, Span, State,
    // Confirmed, Token).
    let row_el = wd
        .wait_for_element(row_a, Duration::from_secs(10))
        .await
        .expect("row A");
    wd.click(&row_el)
        .await
        .expect("click row A opens inspector");
    wd.wait_for_element("aside", Duration::from_secs(10))
        .await
        .expect("inspector drawer");
    let deadline = Instant::now() + Duration::from_secs(20);
    let (states, confirmed_sum) = loop {
        let mut states = Vec::new();
        let mut confirmed = Vec::new();
        for i in 1..=64 {
            let Some(state_cell) = wd
                .wait_for_element(
                    &format!("aside table tbody tr:nth-child({i}) td:nth-child(3)"),
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
                    &format!("aside table tbody tr:nth-child({i}) td:nth-child(4)"),
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

    // Server truth: the recorder holds the whole object.
    let (puts_a, bytes_a) = read_record(&env.record_a).await.expect("record A");
    assert!(puts_a >= 1, "recorder saw at least one PUT");
    assert_eq!(
        bytes_a,
        std::fs::metadata(&env.file_a).unwrap().len(),
        "recorder received the full object"
    );
    let head = head_status(&env.endpoint_a).await.expect("HEAD endpoint A");
    assert_eq!(head, 200, "done job → object present");

    // ---- leg B: cancel mid-transfer → Canceled badge → NO server object ----
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

    let row_b = "table tbody tr:last-child";
    wait_state(&wd, row_b, "running", Duration::from_secs(30), None)
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
    wait_state(&wd, row_b, "canceled", Duration::from_secs(30), None)
        .await
        .expect("job B canceled");

    tokio::time::sleep(Duration::from_millis(500)).await; // let the recorder settle
    let (_, bytes_b) = read_record(&env.record_b).await.expect("record B");
    assert_eq!(
        bytes_b, 0,
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
