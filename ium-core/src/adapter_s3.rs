//! S3 multipart adapter — the first real transport (issue #7, docs/07 §1–§5).
//!
//! Protocol mapping (design note on the issue, rehearsed live against Garage
//! v2.1.0 before implementation — every behavior below is observed wire fact,
//! not spec assumption):
//!
//! * `probe` → `CreateMultipartUpload` (`POST /{bucket}/{key}?uploads`);
//!   `session_token = UploadId`. **0-byte sources open no session** (S3
//!   rejects 0-byte parts; a phantom session would be an orphan) — the
//!   `[(0,0)]` lease pinned by issue #5 maps to one honest `PutObject`
//!   round-trip instead.
//! * `upload_chunk` → `UploadPart` (`PUT ...?partNumber={idx+1}&uploadId=U`),
//!   body streamed from disk over **`[lease.begin, lease.end)`** — the full
//!   chunk span, not `[resume_from, …)`: parts are immutable once uploaded,
//!   and S3 confirms 0 or the full span, so `resume_from == begin` holds for
//!   every S3 lease by construction (spike §3). `bytes_confirmed` is the full
//!   span or an error — never a mid-part number.
//! * `ETag` → `adapter_token = "N:etag"` (quoted, as returned). Response
//!   headers arrive lowercased from hyper-based servers; reads go through
//!   reqwest's case-insensitive `HeaderMap` (the rehearsal's first Complete
//!   failed exactly on a case-sensitive lookup — pinned by a test).
//! * `verify` = finalize-if-needed + verify (**ADR-010, accepted**):
//!   `ListParts` (paginated `part-number-marker` loop) — 404 `NoSuchUpload`
//!   means already finalized (crash-after-complete) → skip to `HEAD`;
//!   otherwise reconcile (parts contiguous `1..=k` AND Σ sizes ==
//!   `source_size`; a wrong assembly is never Completed) →
//!   `CompleteMultipartUpload` with the server-listed ETags (real-SHA256 XML;
//!   `NoSuchUpload` here = already finalized) → `HEAD` with
//!   `Content-Length == source_size`.
//! * Session state: the trait carries no session parameter (no trait churn —
//!   ADR-010 discussion). The UploadId lives in an in-memory map keyed by
//!   `JobSpec.endpoint`; the durable copy is the manager's journaled `probe`
//!   event (ADR-002: the journal is the only truth). After a restart the host
//!   replays that event into [`S3MultipartAdapter::restore_session`]. A map
//!   miss in `upload_chunk` is `SessionLost` → policy `Reattach`.
//! * Errors: XML `<Code>` + status → the design-note taxonomy —
//!   `NoSuchUpload` on UploadPart → `ExpiredSession`; 503 → `Throttled`
//!   (S3 `SlowDown`, with or without `Retry-After`); other 5xx → `Transient`;
//!   probe-level `NoSuchBucket`/`NoSuchKey` → `Unresumable`; 403 →
//!   `AuthRequired` on probe / `ExpiredSession` mid-session; transport →
//!   `Transient`.
//! * Signing: path-style, region-scoped SigV4 from the vector-pinned `sigv4`
//!   module; streaming bodies are `UNSIGNED-PAYLOAD`, small XML/empty bodies
//!   carry their real SHA-256. URLs and signatures share one encoder.
//!
//! Endpoint contract: `JobSpec.endpoint` is a path-style object URL
//! `http(s)://host[:port]/{bucket}[/{key}]`; the path is the **raw key** (the
//! adapter S3-encodes it at signing time), defaulting to the source file's
//! basename when only the bucket is present. Credentials are adapter-level
//! config, never per-job: [`S3MultipartConfig::from_env`] reads
//! `IUM_S3_ACCESS_KEY` / `IUM_S3_SECRET_KEY` / `IUM_S3_REGION`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{CONTENT_LENGTH, ETAG, RETRY_AFTER};
use reqwest::{Body, Client, Method, Response, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::adapter::{
    AdapterError, Capability, ChunkConfirm, ProbeReport, ProgressReporter, TransferAdapter,
    VerifyReport,
};
use crate::adapter_http::Counting;
use crate::chunk::Lease;
use crate::sigv4::{sha256_hex, uri_encode, Credentials, PayloadHash, SigV4Signer, SigningInput};
use crate::JobSpec;

/// S3's hard ceiling for one part (the honest `max_chunk_bytes` report; the
/// planner starts consuming it in #8, ADR-011).
pub const S3_MAX_PART_BYTES: u64 = 5 * 1024 * 1024 * 1024;

const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Adapter-level credentials + signing scope. Never derived from job specs.
#[derive(Debug, Clone)]
pub struct S3MultipartConfig {
    /// SigV4 access key id (harness/CI throws one away per run).
    pub access_key: String,
    /// SigV4 secret key.
    pub secret_key: String,
    /// AWS region — scope of the signature (path-style addressing).
    pub region: String,
}

impl S3MultipartConfig {
    /// Harness/production constructor from the `IUM_S3_*` environment
    /// (region defaults to `us-east-1`).
    pub fn from_env() -> Result<Self, AdapterError> {
        let access_key = std::env::var("IUM_S3_ACCESS_KEY")
            .map_err(|_| AdapterError::Unknown("IUM_S3_ACCESS_KEY not set".into()))?;
        let secret_key = std::env::var("IUM_S3_SECRET_KEY")
            .map_err(|_| AdapterError::Unknown("IUM_S3_SECRET_KEY not set".into()))?;
        let region = std::env::var("IUM_S3_REGION").unwrap_or_else(|_| "us-east-1".into());
        Ok(Self {
            access_key,
            secret_key,
            region,
        })
    }
}

/// One completed part as the server listed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedPart {
    /// 1-based part number.
    pub number: u32,
    /// Part size in bytes (reconcile checks Σ sizes == source size).
    pub size: u64,
    /// The server's ETag for the part (quoted, as returned).
    pub etag: String,
}

/// Which call an error happened on — classification differs slightly
/// (probe-level 403/404 are fatal; chunk-level `NoSuchUpload` is an expired
/// session; finalize treats `NoSuchUpload` as idempotent success before
/// classification ever sees it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCtx {
    /// CreateMultipartUpload / pre-flight calls.
    Probe,
    /// UploadPart / PutObject.
    Chunk,
    /// ListParts / CompleteMultipartUpload.
    Finalize,
}

// ---------------------------------------------------------------------------
// pure protocol helpers — pinned by tests in tests/adapter_s3.rs
// ---------------------------------------------------------------------------

/// The target of one S3 operation, parsed from `JobSpec.endpoint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Target {
    /// `host[:port]` — no scheme.
    pub host: String,
    /// HTTPS requested?
    pub https: bool,
    /// Path-style bucket (first path segment).
    pub bucket: String,
    /// Raw object key (remaining path segments; None = bucket-only spec,
    /// key derived from the source file name).
    pub key: String,
}

/// Parse a path-style object URL plus the job's source path into
/// (host, scheme, bucket, key). The endpoint path is the raw key; when it
/// holds only the bucket, the key defaults to the source basename.
pub fn parse_target(endpoint: &str, source_path: &str) -> Result<S3Target, AdapterError> {
    let unknown = |m: &str| AdapterError::Unknown(format!("bad S3 endpoint '{endpoint}': {m}"));
    let (https, rest) = if let Some(r) = endpoint.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = endpoint.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(unknown("scheme must be http(s)://"));
    };
    let (host, path) = rest
        .split_once('/')
        .ok_or_else(|| unknown("no bucket in path"))?;
    if host.is_empty() {
        return Err(unknown("empty host"));
    }
    let (bucket, key) = match path.split_once('/') {
        Some((b, k)) => (b, Some(k)),
        None => (path, None),
    };
    if bucket.is_empty() {
        return Err(unknown("empty bucket"));
    }
    let key = match key {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => std::path::Path::new(source_path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_string())
            .ok_or_else(|| unknown("no key in endpoint and source has no basename"))?,
    };
    Ok(S3Target {
        host: host.to_string(),
        https,
        bucket: bucket.to_string(),
        key,
    })
}

/// `/{bucket}/{encoded/key}` — S3 single-encoding per segment, slashes kept
/// as separators. The same encoder feeds the signature, so wire and canonical
/// request can never drift apart.
pub fn encode_object_uri(target: &S3Target) -> String {
    let key: String = target
        .key
        .split('/')
        .map(uri_encode)
        .collect::<Vec<_>>()
        .join("/");
    format!("/{}/{}", uri_encode(&target.bucket), key)
}

/// `"{part_number}:{etag}"` — the journal's `adapter_token` for a part.
pub fn part_token(idx: u32, etag: &str) -> String {
    format!("{}:{etag}", idx + 1)
}

/// Inverse of [`part_token`] (1-based part number). `None` on any deviation.
pub fn parse_part_token(token: &str) -> Option<(u32, &str)> {
    let (n, etag) = token.split_once(':')?;
    let n = n.parse().ok()?;
    Some((n, etag))
}

/// First `<tag>…</tag>`'s text with XML character references decoded in ONE
/// left-to-right pass (each reference decoded exactly once, so `&amp;#34;`
/// yields the literal text `&#34;`, never `"` — no double decode).
/// Deliberately not an XML library, but BOTH reference shapes real servers
/// emit are handled: the five named entities (`&quot;` — Garage v2.1.0,
/// issue #7 rehearsal) and numeric character references (`&#34;`/`&#x22;` —
/// SeaweedFS 4.51, issue #9: the named-only decode left literal `&#34;` in
/// the etag and Complete answered `InvalidPart`).
pub fn xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(decode_entities(&xml[start..end]))
}

/// One pass over `s`: at every `&`, decode the longest valid reference
/// (named or numeric) exactly once; anything else is copied verbatim.
fn decode_entities(s: &str) -> String {
    const NAMED: [(&str, char); 5] = [
        ("&quot;", '"'),
        ("&apos;", '\''),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&amp;", '&'),
    ];
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if !s[i..].starts_with('&') {
            let ch = s[i..].chars().next().expect("non-empty remainder");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let rest = &s[i..];
        if let Some((ent, ch)) = NAMED.iter().find(|(e, _)| rest.starts_with(e)) {
            out.push(*ch);
            i += ent.len();
            continue;
        }
        if let Some((ch, consumed)) = decode_numeric_ref(rest) {
            out.push(ch);
            i += consumed;
            continue;
        }
        out.push('&');
        i += 1;
    }
    out
}

/// `&#DDDD;` (decimal) or `&#xHHHH;` (hex) → `(char, bytes consumed)`.
fn decode_numeric_ref(s: &str) -> Option<(char, usize)> {
    let rest = s.strip_prefix("&#")?;
    let (body, prefix_len, radix) = match rest.strip_prefix(['x', 'X']) {
        Some(r) => (r, 3, 16),
        None => (rest, 2, 10),
    };
    let end = body.find(';')?;
    let digits = &body[..end];
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    let code = u32::from_str_radix(digits, radix).ok()?;
    char::from_u32(code).map(|c| (c, prefix_len + end + 1))
}

/// XML-escape text for the Complete body (etags contain `"`).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Parse a `ListPartsResult` document: the parts (sorted by number), whether
/// the list is truncated, and the `NextPartNumberMarker` for pagination.
pub fn parse_list_parts(xml: &str) -> (Vec<ListedPart>, bool, Option<String>) {
    let mut parts = Vec::new();
    let mut rest = xml;
    while let Some(p) = rest.find("<Part>") {
        let after = &rest[p + "<Part>".len()..];
        let Some(e) = after.find("</Part>") else {
            break;
        };
        let block = &after[..e];
        if let (Some(n), Some(s), Some(et)) = (
            xml_text(block, "PartNumber").and_then(|v| v.parse().ok()),
            xml_text(block, "Size").and_then(|v| v.parse().ok()),
            xml_text(block, "ETag"),
        ) {
            parts.push(ListedPart {
                number: n,
                size: s,
                etag: et,
            });
        }
        rest = &after[e + "</Part>".len()..];
    }
    parts.sort_by_key(|p| p.number);
    let truncated = xml_text(xml, "IsTruncated").as_deref() == Some("true");
    let marker = xml_text(xml, "NextPartNumberMarker");
    (parts, truncated, marker)
}

/// The finalize gate's reconcile rule: parts must be contiguous `1..=k` and
/// their sizes must sum to exactly `source_size`. Gaps, duplicates, short or
/// over-long assemblies are rejections — `Complete` is never sent for a wrong
/// assembly.
pub fn reconcile_parts(parts: &[ListedPart], source_size: u64) -> Result<(), String> {
    if parts.is_empty() {
        return if source_size == 0 {
            Ok(())
        } else {
            Err(format!(
                "server lists no parts, expected {source_size} bytes"
            ))
        };
    }
    let mut sum = 0u64;
    for (expected, p) in (1u32..).zip(parts.iter()) {
        if p.number != expected {
            return Err(format!(
                "part numbers not contiguous: expected {expected}, found {}",
                p.number
            ));
        }
        sum = sum.saturating_add(p.size);
    }
    if sum != source_size {
        Err(format!(
            "server holds {sum} bytes across parts, expected {source_size}"
        ))
    } else {
        Ok(())
    }
}

/// `CompleteMultipartUpload` body — parts ascending, etags XML-escaped.
pub fn complete_multipart_xml(parts: &[(u32, &str)]) -> String {
    let mut sorted: Vec<(u32, &str)> = parts.to_vec();
    sorted.sort_by_key(|(n, _)| *n);
    let body: String = sorted
        .iter()
        .map(|(n, etag)| {
            format!(
                "<Part><ETag>{}</ETag><PartNumber>{n}</PartNumber></Part>",
                xml_escape(etag)
            )
        })
        .collect();
    format!(
        "<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{body}</CompleteMultipartUpload>"
    )
}

/// Map (context, status, S3 error code, Retry-After) onto the adapter
/// taxonomy — the design-note table, one place, pinned by tests.
pub fn map_error(
    ctx: ErrorCtx,
    status: u16,
    s3_code: Option<&str>,
    retry_after_ms: Option<u64>,
    detail: &str,
) -> AdapterError {
    let detail = match s3_code {
        Some(code) => format!("{detail} (code={code})"),
        None => detail.to_string(),
    };
    if status == 429 || status == 503 {
        return AdapterError::Throttled {
            retry_after_ms,
            message: detail,
        };
    }
    match (ctx, s3_code) {
        (_, Some("NoSuchUpload")) => AdapterError::ExpiredSession(detail),
        (ErrorCtx::Probe, Some("NoSuchBucket")) | (ErrorCtx::Probe, Some("NoSuchKey")) => {
            AdapterError::Unresumable(detail)
        }
        (_, Some("NoSuchBucket")) | (_, Some("NoSuchKey")) => AdapterError::Unknown(detail),
        _ if status == 403 && ctx == ErrorCtx::Probe => AdapterError::AuthRequired(detail),
        _ if status == 403 => AdapterError::ExpiredSession(detail),
        _ if (500..600).contains(&status) => AdapterError::Transient(detail),
        _ => AdapterError::Unknown(detail),
    }
}

// ---------------------------------------------------------------------------
// adapter
// ---------------------------------------------------------------------------

/// The `s3_multipart` transfer adapter. Construct once, share across jobs:
/// it is `Send + Sync` and worker tasks may call `upload_chunk` concurrently
/// (parts of different jobs never collide; a same-job tail race is resolved
/// server-side by last-write-wins — spike §2.3).
pub struct S3MultipartAdapter {
    client: Client,
    signer: SigV4Signer,
    sessions: Mutex<HashMap<String, String>>,
}

/// One S3 request description — method, target, signed query/headers and
/// payload-hash mode. Bundles what used to be eight loose arguments (the
/// same medicine #6's clippy forced on the signer: no allow-escapes).
struct S3Request<'a> {
    ctx: ErrorCtx,
    what: &'a str,
    method: &'a str,
    target: &'a S3Target,
    query: &'a [(&'a str, &'a str)],
    payload: &'a PayloadHash,
    extra: &'a [(&'a str, &'a str)],
}

impl S3MultipartAdapter {
    /// Build the adapter over real SigV4 credentials (the `sigv4` module —
    /// hmac/sha2/hex only, pinned against official AWS vectors, issue #6).
    pub fn new(config: S3MultipartConfig) -> Result<Self, AdapterError> {
        let signer = SigV4Signer::new(
            Credentials {
                access_key: config.access_key,
                secret_key: config.secret_key,
            },
            config.region,
            "s3",
        );
        let client = Client::builder()
            .user_agent(concat!("IUM/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60 * 60))
            .tcp_nodelay(true)
            .build()
            .map_err(|e| AdapterError::Unknown(format!("http client build failed: {e}")))?;
        Ok(Self {
            client,
            signer,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// Reattach path after a restart: the host reads the UploadId from the
    /// journaled `probe` event (ADR-002) and replays it here. The kill-9
    /// drill (#9) exercises exactly this.
    pub fn restore_session(&self, endpoint: &str, upload_id: String) {
        self.sessions
            .lock()
            .unwrap()
            .insert(endpoint.to_string(), upload_id);
    }

    /// Current session token for an endpoint (probe/restore state) — used by
    /// tests and the future CLI to assert live-vs-journaled consistency.
    pub fn session_for(&self, endpoint: &str) -> Option<String> {
        self.sessions.lock().unwrap().get(endpoint).cloned()
    }

    /// Sign one request and send it. `req` carries everything that
    /// participates in the signature; `body` is the request body itself.
    async fn send(&self, req: &S3Request<'_>, body: Body) -> Result<Response, AdapterError> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| AdapterError::Unknown(format!("bad method '{}': {e}", req.method)))?;
        let target = req.target;
        let uri = encode_object_uri(target);

        // The signer canonicalizes query/headers itself; the wire URL must
        // use the same encoding, so build it from the same encoder.
        let wire_query: Vec<String> = req
            .query
            .iter()
            .map(|(k, v)| format!("{}={}", uri_encode(k), uri_encode(v)))
            .collect();
        let query_owned: Vec<(String, String)> = req
            .query
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let query_ref: Vec<(&str, &str)> = query_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let extra_owned: Vec<(String, String)> = req
            .extra
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let extra_ref: Vec<(&str, &str)> = extra_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let (date, datetime) = amz_now();
        let signed = self.signer.sign(SigningInput {
            method: method.as_str(),
            host: &target.host,
            canonical_uri: &uri,
            query: &query_ref,
            extra_headers: &extra_ref,
            payload: req.payload,
            date: &date,
            datetime: &datetime,
        });

        let mut url = format!(
            "{}://{}{}",
            if target.https { "https" } else { "http" },
            target.host,
            uri
        );
        if !wire_query.is_empty() {
            url.push('?');
            url.push_str(&wire_query.join("&"));
        }

        self.client
            .request(method, &url)
            .header("Authorization", &signed.authorization)
            .header("x-amz-date", &signed.x_amz_date)
            .header("x-amz-content-sha256", &signed.x_amz_content_sha256)
            .headers(extra_headers(req.extra)?)
            .body(body)
            .send()
            .await
            .map_err(|e| AdapterError::Transient(format!("transport: {e}")))
    }

    /// Status→error extraction shared by all call sites: Retry-After header,
    /// S3 `<Code>` from the error body, then the taxonomy.
    async fn classify_failure(
        ctx: ErrorCtx,
        what: &str,
        status: StatusCode,
        retry_after_ms: Option<u64>,
        body: String,
    ) -> AdapterError {
        let code = xml_text(&body, "Code");
        map_error(
            ctx,
            status.as_u16(),
            code.as_deref(),
            retry_after_ms,
            &format!("{what} returned {status}"),
        )
    }

    /// One request round-trip with error mapping. `Ok` responses are 2xx.
    async fn call(&self, req: S3Request<'_>, body: Body) -> Result<Response, AdapterError> {
        let resp = self.send(&req, body).await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let retry_after = retry_after_ms(&resp);
        let text = resp.text().await.unwrap_or_default();
        Err(Self::classify_failure(req.ctx, req.what, status, retry_after, text).await)
    }

    /// Finalize-flavored round-trip implementing ADR-010 idempotency:
    /// `Ok(Some(resp))` = 2xx; `Ok(None)` = the server answered
    /// `NoSuchUpload`, i.e. the multipart upload is already finalized
    /// (crash-after-complete); `Err` = anything else, classified.
    async fn call_finalize(
        &self,
        req: S3Request<'_>,
        body: Body,
    ) -> Result<Option<Response>, AdapterError> {
        let resp = self.send(&req, body).await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(Some(resp));
        }
        let retry_after = retry_after_ms(&resp);
        let text = resp.text().await.unwrap_or_default();
        if xml_text(&text, "Code").as_deref() == Some("NoSuchUpload") {
            return Ok(None);
        }
        Err(Self::classify_failure(ErrorCtx::Finalize, req.what, status, retry_after, text).await)
    }

    /// Signed `HEAD` — the final gate of ADR-010's verify. Returns a report
    /// rather than an error: a HEAD that cannot confirm is an honest
    /// `ok: false` with the reason (mirrors the HTTP adapter).
    async fn head_report(&self, target: &S3Target) -> VerifyReport {
        let payload = PayloadHash::Sha256Hex(EMPTY_SHA.into());
        let req = S3Request {
            ctx: ErrorCtx::Finalize,
            what: "HEAD",
            method: "HEAD",
            target,
            query: &[],
            payload: &payload,
            extra: &[],
        };
        let resp = match self.send(&req, Body::from(Vec::new())).await {
            Ok(r) => r,
            Err(e) => {
                return VerifyReport {
                    ok: false,
                    detail: Some(format!("HEAD failed: {e}")),
                };
            }
        };
        let status = resp.status();
        if !status.is_success() {
            return VerifyReport {
                ok: false,
                detail: Some(format!("HEAD returned {status}")),
            };
        }
        // Read the Content-Length HEADER, not `resp.content_length()`: a HEAD
        // body is empty by definition and hyper would yield zero (same trap
        // as the HTTP adapter).
        match resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            Some(n) => VerifyReport {
                ok: true,
                detail: Some(format!("server holds all {n} bytes")),
            },
            None => VerifyReport {
                ok: false,
                detail: Some("HEAD response carries no content-length".into()),
            },
        }
    }

    /// Diagnostic/test helper: signed GET that buffers the whole object, for
    /// read-back integrity (bytes, not just length). NOT a production path —
    /// do not stream large objects through it.
    pub async fn test_get_object(&self, spec: &JobSpec) -> Result<Vec<u8>, AdapterError> {
        let target = parse_target(&spec.endpoint, &spec.source_path)?;
        let payload = PayloadHash::Sha256Hex(EMPTY_SHA.into());
        let req = S3Request {
            ctx: ErrorCtx::Finalize,
            what: "GET",
            method: "GET",
            target: &target,
            query: &[],
            payload: &payload,
            extra: &[],
        };
        let resp = self.call(req, Body::from(Vec::new())).await?;
        Ok(resp
            .bytes()
            .await
            .map_err(|e| AdapterError::Transient(format!("read: {e}")))?
            .to_vec())
    }
}

fn retry_after_ms(resp: &Response) -> Option<u64> {
    resp.headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

fn extra_headers(extra: &[(&str, &str)]) -> Result<reqwest::header::HeaderMap, AdapterError> {
    let mut map = reqwest::header::HeaderMap::new();
    for (k, v) in extra {
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| AdapterError::Unknown(format!("bad header name '{k}': {e}")))?;
        let value = reqwest::header::HeaderValue::from_str(v)
            .map_err(|e| AdapterError::Unknown(format!("bad header value for '{k}': {e}")))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn io_err(e: std::io::Error) -> AdapterError {
    AdapterError::Transient(format!("local io: {e}"))
}

/// `x-amz-date` ingredients from the system clock: (date, datetime) in UTC.
fn amz_now() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    civil_from_epoch(secs)
}

/// Civil-date conversion (Howard Hinnant's algorithm) — no chrono dep.
/// Returns (`YYYYMMDD`, `YYYYMMDDTHHMMSSZ`).
fn civil_from_epoch(secs: u64) -> (String, String) {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    (
        format!("{y:04}{mth:02}{d:02}"),
        format!(
            "{y:04}{mth:02}{d:02}T{h:02}{mi:02}{s:02}Z",
            h = rem / 3600,
            mi = (rem % 3600) / 60,
            s = rem % 60
        ),
    )
}

#[async_trait]
impl TransferAdapter for S3MultipartAdapter {
    fn name(&self) -> &'static str {
        "s3_multipart"
    }

    fn capabilities(&self) -> Capability {
        // Resume is PART-granularity (parts are atomic) — the badge must not
        // overpromise byte-resume (ADR-004 honesty).
        Capability::new(Capability::RESUMABLE | Capability::DYNAMIC_CHUNKS)
    }

    async fn probe(&self, spec: &JobSpec) -> Result<ProbeReport, AdapterError> {
        // 0-byte sources cannot use multipart at all (S3 rejects 0-byte
        // parts). Open NO session — a phantom UploadId would be an orphan
        // server-side. The honest report: re-runnable (an empty PUT is
        // idempotent), no session token, planner unconstrained.
        if spec.source_size == 0 {
            return Ok(ProbeReport {
                resumable: true,
                max_chunk_bytes: None,
                max_parallel: None,
                session_token: None,
            });
        }
        let target = parse_target(&spec.endpoint, &spec.source_path)?;
        let payload = PayloadHash::Sha256Hex(EMPTY_SHA.into());
        let req = S3Request {
            ctx: ErrorCtx::Probe,
            what: "CreateMultipartUpload",
            method: "POST",
            target: &target,
            query: &[("uploads", "")],
            payload: &payload,
            extra: &[],
        };
        let resp = self.call(req, Body::from(Vec::new())).await?;
        let body = resp
            .text()
            .await
            .map_err(|e| AdapterError::Transient(format!("read: {e}")))?;
        let upload_id = xml_text(&body, "UploadId").ok_or_else(|| {
            AdapterError::Unknown("CreateMultipartUpload: no UploadId in response".into())
        })?;
        self.sessions
            .lock()
            .unwrap()
            .insert(spec.endpoint.clone(), upload_id.clone());
        Ok(ProbeReport {
            resumable: true,
            max_chunk_bytes: Some(S3_MAX_PART_BYTES),
            // Parts are independent resources — no parallelism constraint.
            max_parallel: None,
            session_token: Some(upload_id),
        })
    }

    async fn upload_chunk(
        &self,
        spec: &JobSpec,
        lease: &Lease,
        progress: &ProgressReporter,
    ) -> Result<ChunkConfirm, AdapterError> {
        let target = parse_target(&spec.endpoint, &spec.source_path)?;

        // Source must be exactly what the journal believes (a changed source
        // is Corruption, not silent loss) — same discipline as the HTTP
        // adapter.
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

        // 0-byte job: the [(0,0)] lease is one honest PutObject round-trip.
        // Content-Length: 0 is REQUIRED on the wire (issue #36): hyper sends
        // neither Content-Length nor chunked-TE for an empty fixed-size body,
        // and AWS S3 semantics — which rclone/gofakes3 enforces with 411
        // MissingContentLength where SeaweedFS happened to be lenient — treat
        // a PutObject without a length as invalid. Same mechanism UploadPart
        // already uses below.
        if spec.source_size == 0 {
            let payload = PayloadHash::Sha256Hex(EMPTY_SHA.into());
            let extra = [("content-length", "0")];
            let req = S3Request {
                ctx: ErrorCtx::Chunk,
                what: "PutObject",
                method: "PUT",
                target: &target,
                query: &[],
                payload: &payload,
                extra: &extra,
            };
            let resp = self.call(req, Body::from(Vec::new())).await?;
            let etag = etag_of(&resp)
                .ok_or_else(|| AdapterError::Unknown("PutObject returned no ETag".into()))?;
            return Ok(ChunkConfirm {
                idx: lease.idx,
                bytes_confirmed: 0,
                adapter_token: Some(format!("PUT:{etag}")),
            });
        }

        // Multipart path: the FULL chunk span (parts are atomic — module
        // doc). Wire bytes counted via the Counting reader; bytes_confirmed
        // is the whole span or an error.
        let span = lease.end - lease.begin;
        let upload_id = self
            .sessions
            .lock()
            .unwrap()
            .get(&spec.endpoint)
            .cloned()
            .ok_or_else(|| {
                AdapterError::SessionLost(
                    "no multipart session for endpoint; restore from the journaled probe event"
                        .into(),
                )
            })?;
        let part_number = (lease.idx + 1).to_string();

        file.seek(std::io::SeekFrom::Start(lease.begin))
            .await
            .map_err(io_err)?;
        let reader = file.take(span);
        let body = Body::wrap_stream(FramedRead::new(
            Counting {
                inner: reader,
                progress: progress.clone(),
            },
            BytesCodec::new(),
        ));

        let payload = PayloadHash::Unsigned;
        let extra = [("content-length", span.to_string())];
        let extra_refs: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let query = [
            ("partNumber", part_number.as_str()),
            ("uploadId", upload_id.as_str()),
        ];
        let req = S3Request {
            ctx: ErrorCtx::Chunk,
            what: "UploadPart",
            method: "PUT",
            target: &target,
            query: &query,
            payload: &payload,
            extra: &extra_refs,
        };
        let resp = self.call(req, body).await?;
        let etag = etag_of(&resp)
            .ok_or_else(|| AdapterError::Unknown("UploadPart returned no ETag".into()))?;

        Ok(ChunkConfirm {
            idx: lease.idx,
            bytes_confirmed: span,
            adapter_token: Some(part_token(lease.idx, &etag)),
        })
    }

    /// Reattach (issue #12): replay the journaled probe event's UploadId
    /// into the session map. Pure in-memory restore — no network, no new
    /// multipart upload (a re-probe's CreateMultipartUpload would orphan the
    /// live session). The kill-9 drill (#9) already exercised this path via
    /// the inherent `restore_session`; the trait method is the same door,
    /// promised by the taxonomy instead of host-side special-casing.
    async fn reattach(&self, endpoint: &str, token: &str) -> Result<(), AdapterError> {
        self.restore_session(endpoint, token.to_string());
        Ok(())
    }

    async fn verify(&self, spec: &JobSpec) -> Result<VerifyReport, AdapterError> {
        let target = parse_target(&spec.endpoint, &spec.source_path)?;

        // 0-byte: no session exists — HEAD is the whole gate.
        if spec.source_size == 0 {
            let report = self.head_report(&target).await;
            if report.ok {
                return Ok(report);
            }
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "0-byte object not confirmed: {}",
                    report.detail.unwrap_or_default()
                )),
            });
        }

        let upload_id = self.session_for(&spec.endpoint).ok_or_else(|| {
            AdapterError::SessionLost(
                "no multipart session for endpoint; restore from the journaled probe event".into(),
            )
        })?;

        // -- 1. ListParts, paginated (part-number-marker loop: unreachable
        // under CHUNK_MAX × 1000, implemented for correctness — spike §8).
        let mut all_parts: Vec<ListedPart> = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let marker_string;
            let mut query: Vec<(&str, &str)> = vec![("uploadId", upload_id.as_str())];
            if let Some(m) = &marker {
                marker_string = m.clone();
                query.push(("part-number-marker", marker_string.as_str()));
            }
            let payload = PayloadHash::Sha256Hex(EMPTY_SHA.into());
            let req = S3Request {
                ctx: ErrorCtx::Finalize,
                what: "ListParts",
                method: "GET",
                target: &target,
                query: &query,
                payload: &payload,
                extra: &[],
            };
            match self.call_finalize(req, Body::from(Vec::new())).await? {
                // NoSuchUpload on ListParts = already finalized: skip to HEAD.
                None => {
                    let head = self.head_report(&target).await;
                    return Ok(VerifyReport {
                        ok: head.ok,
                        detail: head.detail.map(|d| {
                            if head.ok {
                                d
                            } else {
                                format!("already finalized; {d}")
                            }
                        }),
                    });
                }
                Some(resp) => {
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| AdapterError::Transient(format!("read: {e}")))?;
                    let (parts, truncated, next) = parse_list_parts(&body);
                    all_parts.extend(parts);
                    if !truncated {
                        break;
                    }
                    marker = Some(next.ok_or_else(|| {
                        AdapterError::Unknown(
                            "ListParts truncated without NextPartNumberMarker".into(),
                        )
                    })?);
                }
            }
        }

        // -- 2. Reconcile BEFORE completing — never Complete a wrong assembly.
        if let Err(detail) = reconcile_parts(&all_parts, spec.source_size) {
            // The refusal carries the server's wire truth (bounded): what the
            // ListParts actually held when the assembly was rejected.
            let listed: String = if all_parts.len() > 32 {
                let head: Vec<String> = all_parts
                    .iter()
                    .take(32)
                    .map(|p| format!("(p{} {}B)", p.number, p.size))
                    .collect();
                format!("{} …", head.join(" "))
            } else {
                all_parts
                    .iter()
                    .map(|p| format!("(p{} {}B)", p.number, p.size))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            return Ok(VerifyReport {
                ok: false,
                detail: Some(format!(
                    "reconcile failed, upload left incomplete: {detail}; server listed {} part(s): {}",
                    all_parts.len(),
                    listed
                )),
            });
        }

        // -- 3. CompleteMultipartUpload with the SERVER-LISTED etags (the
        // server, not the journal, is the truth about each part).
        let refs: Vec<(u32, &str)> = all_parts
            .iter()
            .map(|p| (p.number, p.etag.as_str()))
            .collect();
        let xml = complete_multipart_xml(&refs);
        let payload = PayloadHash::Sha256Hex(sha256_hex(xml.as_bytes()));
        let query = [("uploadId", upload_id.as_str())];
        let req = S3Request {
            ctx: ErrorCtx::Finalize,
            what: "CompleteMultipartUpload",
            method: "POST",
            target: &target,
            query: &query,
            payload: &payload,
            extra: &[],
        };
        // NoSuchUpload on Complete = already completed (ADR-010 idempotency).
        self.call_finalize(req, Body::from(xml)).await?;

        // -- 4. HEAD must show the object at the source size.
        let head = self.head_report(&target).await;
        if head.ok {
            return Ok(head);
        }
        Ok(VerifyReport {
            ok: false,
            detail: Some(format!(
                "post-complete HEAD mismatch: {}",
                head.detail.unwrap_or_default()
            )),
        })
    }
}

fn etag_of(resp: &Response) -> Option<String> {
    // Case-insensitive by construction: Garage/hyper lowercases headers on
    // the wire; HeaderMap lookups do not care.
    resp.headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_conversion_matches_known_timestamps() {
        assert_eq!(civil_from_epoch(0).1, "19700101T000000Z");
        assert_eq!(civil_from_epoch(86400).1, "19700102T000000Z");
        assert_eq!(
            civil_from_epoch(951825600).1,
            "20000229T120000Z",
            "leap day"
        );
        assert_eq!(civil_from_epoch(1735689599).1, "20241231T235959Z");
        assert_eq!(civil_from_epoch(1793580912).0, "20261102");
        assert_eq!(civil_from_epoch(1793580912).1, "20261102T005512Z");
    }

    #[test]
    fn object_uri_encodes_segments_keeps_slashes() {
        let t = S3Target {
            host: "h:9000".into(),
            https: false,
            bucket: "b".into(),
            key: "a+b/c d.txt".into(),
        };
        assert_eq!(encode_object_uri(&t), "/b/a%2Bb/c%20d.txt");
    }

    #[test]
    fn reconcile_empty_zero_is_ok() {
        assert_eq!(reconcile_parts(&[], 0), Ok(()));
    }

    #[test]
    fn map_error_is_transparent_about_codes() {
        let e = map_error(ErrorCtx::Chunk, 404, Some("NoSuchUpload"), None, "d");
        assert!(e.to_string().contains("code=NoSuchUpload"));
    }
}
