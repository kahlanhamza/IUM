//! `check_credentials` — the S3 Test button's engine arm (the destination
//! lane, owner-ratified 2026-11-11). Today `probe_credentials` answers the
//! S3 lane with the honest catch-all "Not checked here — this lane
//! verifies itself on the first upload": the user discovers a wrong URL or
//! rejected keys only AFTER configuring a job. This test pins the fix:
//! a zero-byte `CreateMultipartUpload` + best-effort `AbortMultipartUpload`
//! against a probe key, verdict classified through the adapter's own
//! error taxonomy.
//!
//! RED artifact: at the pre-implementation HEAD this file FAILS TO
//! COMPILE — the method does not exist yet.
//!
//! The mock is the `common/tus_mock.rs` precedent applied to S3: a
//! loopback TCP server that answers the two calls the probe makes, with
//! honest shapes (XML bodies carrying Content-Length; the deny script
//! returns real S3 error XML). It records the wire so the test can assert
//! the probe NEVER touched a user key and always aborted its session.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use ium_core::adapter::AdapterError;
use ium_core::adapter_s3::{S3MultipartAdapter, S3MultipartConfig};

const TEST_AK: &str = "ium-test-key";
const TEST_SK: &str = "ium-test-secret";

fn adapter() -> S3MultipartAdapter {
    S3MultipartAdapter::new(S3MultipartConfig {
        access_key: TEST_AK.into(),
        secret_key: TEST_SK.into(),
        region: "us-east-1".into(),
    })
    .expect("test adapter builds")
}

struct MockState {
    creations: AtomicU64,
    aborts: AtomicU64,
    last_create_path: Mutex<String>,
    last_create_auth: Mutex<String>,
    last_abort_path: Mutex<String>,
    /// When set, the next CreateMultipartUpload answers with this status
    /// and an S3 error XML carrying this `<Code>`.
    deny: Mutex<Option<(u16, String)>>,
}

pub struct S3Mock {
    pub addr: SocketAddr,
    state: Arc<MockState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl S3Mock {
    pub fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(MockState {
            creations: AtomicU64::new(0),
            aborts: AtomicU64::new(0),
            last_create_path: Mutex::new(String::new()),
            last_create_auth: Mutex::new(String::new()),
            last_abort_path: Mutex::new(String::new()),
            deny: Mutex::new(None),
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let st = state.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("mock runtime");
            runtime.block_on(async move {
                listener
                    .set_nonblocking(true)
                    .expect("nonblocking listener");
                let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                let mut shutdown = rx;
                loop {
                    tokio::select! {
                        _ = &mut shutdown => break,
                        accepted = listener.accept() => {
                            let Ok((stream, _)) = accepted else { continue };
                            let st = st.clone();
                            tokio::spawn(async move { handle(stream, st).await });
                        }
                    }
                }
            });
        });
        S3Mock {
            addr,
            state,
            shutdown: Some(tx),
        }
    }

    /// A bucket-only endpoint on the mock (the create form's provider
    /// picker derives exactly this shape; the engine derives the key from
    /// the source basename at job time).
    pub fn endpoint(&self, bucket: &str) -> String {
        format!(
            "http://{}{}",
            self.addr,
            format_args!("/{}", bucket.trim_start_matches('/'))
        )
    }

    pub fn set_deny(&self, status: u16, code: &str) {
        *self.state.deny.lock().unwrap() = Some((status, code.to_string()));
    }

    pub fn creations(&self) -> u64 {
        self.state.creations.load(Ordering::SeqCst)
    }

    pub fn aborts(&self) -> u64 {
        self.state.aborts.load(Ordering::SeqCst)
    }

    pub fn last_create_path(&self) -> String {
        self.state.last_create_path.lock().unwrap().clone()
    }

    pub fn last_create_auth(&self) -> String {
        self.state.last_create_auth.lock().unwrap().clone()
    }

    pub fn last_abort_path(&self) -> String {
        self.state.last_abort_path.lock().unwrap().clone()
    }
}

impl Drop for S3Mock {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn handle(mut stream: TcpStream, state: Arc<MockState>) {
    // Read the head (through the blank line) — bodies on these calls are
    // empty, nothing to drain.
    let mut buf = Vec::with_capacity(2048);
    let mut byte = [0u8; 1];
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(10), stream.read(&mut byte)).await
        {
            Ok(Ok(0)) | Err(_) => return,
            Ok(Ok(_)) => {}
            // A read error mid-head: treat like EOF, the client is gone.
            Ok(Err(_)) => return,
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let auth = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            if !name.eq_ignore_ascii_case("Authorization") {
                return None;
            }
            Some(value.trim().to_string())
        })
        .unwrap_or_default();

    if method == "POST" && path.contains("uploads") {
        // Clone the deny script first — the std MutexGuard must drop
        // BEFORE any await (tokio tasks need Send).
        let deny = state.deny.lock().unwrap().clone();
        if let Some((status, code)) = deny {
            let body = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code>\
                 <Message>the mock refused this probe</Message></Error>"
            );
            let resp = format!(
                "HTTP/1.1 {status} Denied\r\nConnection: close\r\nContent-Type: application/xml\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).await.expect("write deny");
            return;
        }
        state.creations.fetch_add(1, Ordering::SeqCst);
        *state.last_create_path.lock().unwrap() = path.clone();
        *state.last_create_auth.lock().unwrap() = auth;
        let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CreateMultipartUploadResult>\
                    <Bucket>mock</Bucket><Key>mock</Key>\
                    <UploadId>ium-probe-upload-0</UploadId></CreateMultipartUploadResult>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/xml\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(resp.as_bytes())
            .await
            .expect("write create");
        return;
    }
    if method == "DELETE" && path.contains("uploadId=") {
        state.aborts.fetch_add(1, Ordering::SeqCst);
        *state.last_abort_path.lock().unwrap() = path.clone();
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .await
            .expect("write abort");
        return;
    }
    let _ = stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
        .await;
}

// ---------------------------------------------------------------------------
// the contracts
// ---------------------------------------------------------------------------

/// The happy path: the probe creates ONE session on a probe key (never a
/// user key — the path must carry the `ium-probe/` namespace), aborts it,
/// and the request was signed (an Authorization header rode the wire).
#[tokio::test]
async fn happy_path_creates_and_aborts_on_a_probe_key() {
    let mock = S3Mock::spawn();
    let result = adapter()
        .check_credentials(&mock.endpoint("ium-bucket"))
        .await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");
    assert_eq!(mock.creations(), 1, "exactly one CreateMultipartUpload");
    assert_eq!(
        mock.aborts(),
        1,
        "exactly one AbortMultipartUpload — no orphan"
    );
    let path = mock.last_create_path();
    assert!(
        path.starts_with("/ium-bucket/ium-probe/"),
        "the probe key must live under the bucket's ium-probe/ namespace, got '{path}'"
    );
    assert!(
        mock.last_create_auth().starts_with("AWS4-HMAC-SHA256"),
        "the probe request must be signed"
    );
    assert!(
        mock.last_abort_path()
            .contains("uploadId=ium-probe-upload-0"),
        "the abort must target the created session"
    );
}

/// A bucket-only endpoint (no key) is the NORMAL shape — the picker
/// derives it. The probe works without the user inventing a key.
#[tokio::test]
async fn bucket_only_endpoints_are_probeable() {
    let mock = S3Mock::spawn();
    let url = format!("http://{}/just-a-bucket", mock.addr);
    let result = adapter().check_credentials(&url).await;
    assert!(
        result.is_ok(),
        "bucket-only endpoint must probe fine, got {result:?}"
    );
}

/// Rejected keys: a probe-context 403 classifies as AuthRequired — the
/// same taxonomy the upload path uses, so the UI's copy table speaks one
/// language.
#[tokio::test]
async fn rejected_keys_surface_auth_required() {
    let mock = S3Mock::spawn();
    mock.set_deny(403, "AccessDenied");
    let result = adapter()
        .check_credentials(&mock.endpoint("ium-bucket"))
        .await;
    assert!(
        matches!(result, Err(AdapterError::AuthRequired(_))),
        "403 must classify AuthRequired, got {result:?}"
    );
    // The probe still cleaned up after itself: deny does not skip the abort.
    assert_eq!(mock.aborts(), 0, "no session was created, none to abort");
}

/// A missing bucket: probe-context `NoSuchBucket` classifies Unresumable
/// (map_error's Probe arm) — an honest "this destination does not exist".
#[tokio::test]
async fn missing_bucket_is_unresumable() {
    let mock = S3Mock::spawn();
    mock.set_deny(404, "NoSuchBucket");
    let result = adapter().check_credentials(&mock.endpoint("nope")).await;
    assert!(
        matches!(result, Err(AdapterError::Unresumable(_))),
        "404 NoSuchBucket must classify Unresumable, got {result:?}"
    );
}

/// A dead endpoint (connection refused) is a transport Transient — the
/// test drops the mock before probing to pin the classification.
#[tokio::test]
async fn dead_endpoint_is_transient() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener); // the port is now closed
    let url = format!("http://{addr}/some-bucket");
    let result = adapter().check_credentials(&url).await;
    assert!(
        matches!(result, Err(AdapterError::Transient(_))),
        "connection refused must classify Transient, got {result:?}"
    );
}

/// A malformed endpoint is refused BEFORE any wire traffic — the user
/// learns the URL shape is wrong, not a vague transport error.
#[tokio::test]
async fn malformed_endpoint_refused_without_wire() {
    let mock = S3Mock::spawn();
    let result = adapter().check_credentials("ftp://not-s3/bucket").await;
    assert!(result.is_err(), "non-http scheme must be refused");
    assert_eq!(mock.creations(), 0, "no wire traffic for a malformed URL");
}
