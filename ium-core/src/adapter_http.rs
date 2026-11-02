//! HTTP one-shot adapter — the honest baseline protocol (spec §5):
//! a single `PUT` of the whole resource. Physically cannot resume: if the
//! connection dies mid-body, the bytes are gone and the job restarts. The
//! capability flags say so (`Capability::default()` → badge `one_shot`), and
//! the supervisor plans such jobs as a single chunk.
//!
//! Protocol details that matter:
//! * body streams straight from disk (`resume_from .. lease.end`), bounded by
//!   `AsyncReadExt::take` — memory use is the socket buffer, not the file;
//! * the local file is size-checked against the journal before the first byte
//!   moves (a source that changed under us is `Corruption`, not silent loss);
//! * error classification per spec §5: 429 → `Throttled` (honoring
//!   `Retry-After`), 403/410 → `ExpiredSession`, 5xx → `Transient`;
//! * verify-before-declare (pattern #10): a `HEAD` must show the server holds
//!   exactly `source_size` bytes before the job may be declared `Done`.
//!   Endpoints that reject `HEAD` need a protocol-specific adapter — the
//!   honesty is the point.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{ETAG, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, ReadBuf};
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::chunk::Lease;
use crate::JobSpec;

pub struct HttpOneShotAdapter {
    client: Client,
}

impl HttpOneShotAdapter {
    pub fn new() -> Result<Self, AdapterError> {
        Self::with_client_config(Duration::from_secs(10), Duration::from_secs(60 * 60))
    }

    /// Test/loopback constructor with shorter timeouts.
    pub fn with_client_config(
        connect_timeout: Duration,
        total_timeout: Duration,
    ) -> Result<Self, AdapterError> {
        let client = Client::builder()
            .user_agent(concat!("IUM/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect_timeout)
            .timeout(total_timeout)
            .tcp_nodelay(true)
            .build()
            .map_err(|e| AdapterError::Unknown(format!("http client build failed: {e}")))?;
        Ok(Self { client })
    }
}

/// Map a failure status to the spec's error taxonomy.
fn classify_status(
    status: StatusCode,
    retry_after: Option<Duration>,
    detail: &str,
) -> AdapterError {
    match status {
        StatusCode::TOO_MANY_REQUESTS => AdapterError::Throttled {
            retry_after_ms: retry_after.map(|d| d.as_millis() as u64),
            message: detail.to_string(),
        },
        StatusCode::FORBIDDEN | StatusCode::GONE => {
            AdapterError::ExpiredSession(detail.to_string())
        }
        s if s.is_server_error() => AdapterError::Transient(detail.to_string()),
        _ => AdapterError::Unknown(detail.to_string()),
    }
}

/// `Retry-After`: delta-seconds or nothing (HTTP-date form needs a clock
/// dependency and is rare on upload endpoints — ignored for now).
fn parse_retry_after(value: &reqwest::header::HeaderValue) -> Option<Duration> {
    let s = value.to_str().ok()?;
    let secs: u64 = s.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

fn io_err(e: std::io::Error) -> AdapterError {
    AdapterError::Transient(format!("local io: {e}"))
}

fn req_err(e: reqwest::Error) -> AdapterError {
    // Transport-level failures (connect, reset, timeout) are transient by
    // definition; the retry policy owns what to do with them.
    AdapterError::Transient(format!("transport: {e}"))
}

/// AsyncRead passthrough that feeds every byte handed to the transport into
/// the call's [`ProgressReporter`] — the supervisor's eviction window (spec
/// §4.2) reads this instead of guessing from timeouts. Shared with the S3
/// adapter (#7), which counts wire bytes the same way.
pub(crate) struct Counting<R> {
    pub(crate) inner: R,
    pub(crate) progress: ProgressReporter,
}

impl<R: AsyncRead + Unpin> AsyncRead for Counting<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let filled_before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let n = (buf.filled().len() - filled_before) as u64;
                if n > 0 {
                    this.progress.add(n);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[async_trait]
impl TransferAdapter for HttpOneShotAdapter {
    fn name(&self) -> &'static str {
        "http_oneshot"
    }

    fn capabilities(&self) -> Capability {
        // No RESUMABLE flag: a dropped PUT body is gone. The UI renders the
        // honest `One-shot ⚠️` badge from exactly this.
        Capability::default()
    }

    async fn probe(&self, _spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        // Nothing to discover for a blind PUT; honesty over ceremony.
        Ok(ProbeReport {
            resumable: false,
            max_chunk_bytes: None,
            max_parallel: None,
            session_token: None,
        })
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let span = lease.end - lease.resume_from;

        let mut file = tokio::fs::File::open(&spec.source_path)
            .await
            .map_err(io_err)?;
        let meta = file.metadata().await.map_err(io_err)?;
        if meta.len() != spec.source_size {
            return Err(AdapterError::Corruption(format!(
                "source changed under the job: journal holds {} bytes, disk has {}",
                spec.source_size,
                meta.len()
            )));
        }
        file.seek(std::io::SeekFrom::Start(lease.resume_from))
            .await
            .map_err(io_err)?;
        let reader = file.take(span);

        let body = reqwest::Body::wrap_stream(FramedRead::new(
            Counting {
                inner: reader,
                progress: progress.clone(),
            },
            BytesCodec::new(),
        ));
        let resp = self
            .client
            .put(&spec.endpoint)
            .body(body)
            .send()
            .await
            .map_err(req_err)?;

        let status = resp.status();
        if !status.is_success() {
            let retry_after = resp.headers().get(RETRY_AFTER).and_then(parse_retry_after);
            return Err(classify_status(
                status,
                retry_after,
                &format!("PUT returned {status}"),
            ));
        }

        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: etag,
        })
    }

    /// Reattach (issue #12): explicit no-op, honestly. A one-shot endpoint
    /// has no session to reattach to, and the manager only calls this when
    /// a journaled probe event carries a `session_token` — which this
    /// adapter's probe never produces (`session_token: None`).
    async fn reattach(&self, _endpoint: &str, _token: &str) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let resp = match self.client.head(&spec.endpoint).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(VerifyReport {
                    ok: false,
                    detail: Some(format!("HEAD failed: {e}")),
                });
            }
        };
        if !resp.status().is_success() {
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!("HEAD returned {}", resp.status())),
            });
        }
        // Read the Content-Length HEADER, not `resp.content_length()`: the
        // latter reports the body size hint, and hyper correctly yields zero
        // for HEAD responses (a HEAD body is empty by definition) — which
        // would make every honest server look like a lying one.
        let declared = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        match declared {
            Some(n) if n == spec.source_size => Ok(VerifyReport {
                ok: true,
                detail: Some(format!("server holds all {n} bytes")),
            }),
            Some(n) => Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "server holds {n} bytes, expected {}",
                    spec.source_size
                )),
            }),
            None => Ok(VerifyReport {
                ok: false,
                detail: Some("HEAD response carries no content-length".to_string()),
            }),
        }
    }
}

/// Method used by this adapter, surfaced for tests/diagnostics.
pub const UPLOAD_METHOD: Method = Method::PUT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_are_parsed() {
        let v = reqwest::header::HeaderValue::from_static("7");
        assert_eq!(parse_retry_after(&v), Some(Duration::from_secs(7)));
        let v = reqwest::header::HeaderValue::from_static(" 0 ");
        assert_eq!(parse_retry_after(&v), Some(Duration::from_secs(0)));
        // HTTP-date form is not supported — policy backoff takes over.
        let v = reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT");
        assert_eq!(parse_retry_after(&v), None);
        let v = reqwest::header::HeaderValue::from_static("soon");
        assert_eq!(parse_retry_after(&v), None);
    }

    #[test]
    fn status_classification_follows_the_taxonomy() {
        use crate::retry::ErrorKind;
        assert_eq!(
            classify_status(StatusCode::TOO_MANY_REQUESTS, None, "d").kind(),
            ErrorKind::Throttled
        );
        assert_eq!(
            classify_status(StatusCode::FORBIDDEN, None, "d").kind(),
            ErrorKind::ExpiredSession
        );
        assert_eq!(
            classify_status(StatusCode::GONE, None, "d").kind(),
            ErrorKind::ExpiredSession
        );
        assert_eq!(
            classify_status(StatusCode::BAD_GATEWAY, None, "d").kind(),
            ErrorKind::Transient
        );
        assert_eq!(
            classify_status(StatusCode::NOT_FOUND, None, "d").kind(),
            ErrorKind::Unknown
        );
    }

    #[test]
    fn client_builds_with_rustls_stack() {
        assert!(HttpOneShotAdapter::new().is_ok());
    }
}
