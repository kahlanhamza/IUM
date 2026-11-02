//! AWS Signature Version 4 request signing — pure, offline, zero I/O.
//!
//! Scope is deliberately narrow: sign the ~6 operations the S3 multipart
//! adapter (#7) needs. No network code, no AWS SDK — the module is provable
//! offline against the official AWS SigV4 test-suite vectors (pinned below,
//! triple-sourced: suite + AWS S3 developer guide + an independent Python
//! reference that had to reproduce every constant before it was pinned).
//!
//! Rules implemented (SigV4):
//! 1. canonical request = method \n canonical-uri \n canonical-query \n
//!    canonical-headers \n signed-headers \n payload-hash
//! 2. string-to-sign = `AWS4-HMAC-SHA256` \n datetime \n scope \n hex(sha256(creq))
//! 3. signing key = HMAC chain `AWS4<secret>` → date → region → service →
//!    `aws4_request`; signature = hex(HMAC(key, sts))
//! 4. **S3-specific rule**: `x-amz-content-sha256` is a signed header only
//!    when `service == "s3"` (the payload hash is always the creq's last
//!    line regardless — this is why generic-service vectors sign
//!    `host;x-amz-date` while S3 vectors sign `host;…;x-amz-content-sha256;…`).
//!
//! The caller passes a pre-encoded canonical URI (S3-style single encoding,
//! no path normalization — the adapter owns the exact path it will hit) and
//! injectable `date`/`datetime` strings (test vectors pin them; the adapter
//! formats the current UTC time itself).

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Endpoint credentials. The caller owns them; this module never stores or
/// logs them beyond the scope of one signing call.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
}

/// How the request body participates in the canonical request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadHash {
    /// Streaming bodies (the upload hot path): sign the literal
    /// `UNSIGNED-PAYLOAD` sentinel. TLS still protects the wire.
    Unsigned,
    /// Small bodies (Create/Complete XML): sign the real SHA-256 hex.
    Sha256Hex(String),
}

/// Headers the caller must attach to the outgoing request, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRequest {
    pub authorization: String,
    pub x_amz_date: String,
    pub x_amz_content_sha256: String,
}

/// SHA-256 hex of bytes — for small bodies (Create/Complete XML) and tests.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// HMAC-SHA256 chain: `AWS4<secret>` → date → region → service → `aws4_request`.
pub fn derived_signing_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    fn hmac(key: &[u8], msg: &str) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(msg.as_bytes());
        let out = mac.finalize().into_bytes();
        let mut block = [0u8; 32];
        block.copy_from_slice(&out);
        block
    }
    let k_date = hmac(format!("AWS4{secret}").as_bytes(), date);
    let k_region = hmac(&k_date, region);
    let k_service = hmac(&k_region, service);
    hmac(&k_service, "aws4_request")
}

/// One complete request description, ready to sign. `canonical_uri` arrives
/// pre-encoded (S3-style single encoding, no path normalization — the caller
/// owns the exact path it will hit); `query` pairs are canonicalized inside
/// (strict RFC3986, byte-wise sort by key then value); `extra_headers`
/// participate in the canonical header set (lowercased, trimmed, sorted)
/// together with `host`, `x-amz-date` and — for the S3 service only —
/// `x-amz-content-sha256`. Extras must not set those three.
/// `date` is `YYYYMMDD` (credential scope); `datetime` is `YYYYMMDDTHHMMSSZ`
/// — injectable so test vectors can pin them.
#[derive(Debug, Clone, Copy)]
pub struct SigningInput<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub canonical_uri: &'a str,
    pub query: &'a [(&'a str, &'a str)],
    pub extra_headers: &'a [(&'a str, &'a str)],
    pub payload: &'a PayloadHash,
    pub date: &'a str,
    pub datetime: &'a str,
}

/// Signs requests for one endpoint (region + service fixed per signer).
#[derive(Debug, Clone)]
pub struct SigV4Signer {
    creds: Credentials,
    region: String,
    service: String,
}

impl SigV4Signer {
    pub fn new(creds: Credentials, region: impl Into<String>, service: impl Into<String>) -> Self {
        Self {
            creds,
            region: region.into(),
            service: service.into(),
        }
    }

    /// Sign one request (see [`SigningInput`] for the field contract).
    pub fn sign(&self, input: SigningInput<'_>) -> SignedRequest {
        let SigningInput {
            method,
            host,
            canonical_uri,
            query,
            extra_headers,
            payload,
            date,
            datetime,
        } = input;
        let payload_hash = match payload {
            PayloadHash::Unsigned => "UNSIGNED-PAYLOAD".to_string(),
            PayloadHash::Sha256Hex(h) => h.clone(),
        };

        // Canonical query: strict-encode, byte-wise sort by key then value.
        let mut pairs: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| (uri_encode(k), uri_encode(v)))
            .collect();
        pairs.sort();
        let canonical_query = pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        // Canonical headers: lowercase names, trimmed values, byte-sorted.
        // host/x-amz-date always; x-amz-content-sha256 for the S3 service
        // only (module doc rule 4). Extras must not set those three — they
        // are inserted first so the fixed ones win on collision.
        let mut headers = std::collections::BTreeMap::new();
        for (k, v) in extra_headers {
            headers.insert(k.to_ascii_lowercase(), v.trim().to_string());
        }
        headers.insert("host".to_string(), host.trim().to_string());
        headers.insert("x-amz-date".to_string(), datetime.to_string());
        if self.service == "s3" {
            headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());
        }
        let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect::<String>();

        // Canonical request → string-to-sign → signature.
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        );
        let scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let key = derived_signing_key(&self.creds.secret_key, date, &self.region, &self.service);
        let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
        mac.update(string_to_sign.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        SignedRequest {
            authorization: format!(
                "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={signed_headers}, Signature={signature}",
                self.creds.access_key, scope
            ),
            x_amz_date: datetime.to_string(),
            x_amz_content_sha256: payload_hash,
        }
    }
}

/// Strict RFC3986 encoding: unreserved `A-Za-z0-9-._~` survive; everything
/// else becomes uppercase `%XX`.
fn uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const AK_GENERIC: &str = "AKIDEXAMPLE";
    const SK_GENERIC: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
    const AK_S3: &str = "AKIAIOSFODNN7EXAMPLE";
    const SK_S3: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn generic_signer() -> SigV4Signer {
        SigV4Signer::new(
            Credentials {
                access_key: AK_GENERIC.into(),
                secret_key: SK_GENERIC.into(),
            },
            "us-east-1",
            "service",
        )
    }

    fn s3_signer() -> SigV4Signer {
        SigV4Signer::new(
            Credentials {
                access_key: AK_S3.into(),
                secret_key: SK_S3.into(),
            },
            "us-east-1",
            "s3",
        )
    }

    /// Vector A — official suite `get-vanilla` (values published in
    /// awslabs/aws-crt-nodejs auth.spec.ts, copied from the aws-c-auth suite).
    #[test]
    fn vector_get_vanilla() {
        let r = generic_signer().sign(SigningInput {
            method: "GET",
            host: "example.amazonaws.com",
            canonical_uri: "/",
            query: &[],
            extra_headers: &[],
            payload: &PayloadHash::Sha256Hex(EMPTY_SHA.into()),
            date: "20150830",
            datetime: "20150830T123600Z",
        });
        assert_eq!(
            r.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    /// Vector B — AWS S3 developer guide, GET object with a `range` header.
    /// Proves extra headers participate; payload = empty-body hash.
    #[test]
    fn vector_s3_get_object_with_range() {
        let r = s3_signer().sign(SigningInput {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            canonical_uri: "/test.txt",
            query: &[],
            extra_headers: &[("range", "bytes=0-9")],
            payload: &PayloadHash::Sha256Hex(EMPTY_SHA.into()),
            date: "20130524",
            datetime: "20130524T000000Z",
        });
        assert_eq!(
            r.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert_eq!(r.x_amz_content_sha256, EMPTY_SHA);
    }

    /// Vector C — AWS S3 developer guide, PUT object with a body.
    /// Proves body-hash mode + header sorting (`date` < `host` < `x-amz-*`)
    /// and the pre-encoded path (`$` → `%24`).
    #[test]
    fn vector_s3_put_object_with_body() {
        let r = s3_signer().sign(SigningInput {
            method: "PUT",
            host: "examplebucket.s3.amazonaws.com",
            canonical_uri: "/test%24file.text",
            query: &[],
            extra_headers: &[
                ("date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ],
            payload: &PayloadHash::Sha256Hex(sha256_hex(b"Welcome to Amazon S3.")),
            date: "20130524",
            datetime: "20130524T000000Z",
        });
        assert_eq!(
            r.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, \
             Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    /// Vector D — suite pair: single query param, canonical `Param1=value1`.
    #[test]
    fn vector_single_query_param() {
        let r = generic_signer().sign(SigningInput {
            method: "GET",
            host: "example.amazonaws.com",
            canonical_uri: "/",
            query: &[("Param1", "value1")],
            extra_headers: &[],
            payload: &PayloadHash::Sha256Hex(EMPTY_SHA.into()),
            date: "20150830",
            datetime: "20150830T123600Z",
        });
        assert_eq!(
            r.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=a67d582fa61cc504c4bae71f336f98b97f1ea3c7a6bfe1b6e45aec72011b9aeb"
        );
    }

    /// Vector E — suite pair: raw params reversed, canonical must sort
    /// byte-wise (`Param1=value1&Param2=value2`).
    #[test]
    fn vector_query_params_are_sorted() {
        let r = generic_signer().sign(SigningInput {
            method: "GET",
            host: "example.amazonaws.com",
            canonical_uri: "/",
            query: &[("Param2", "value2"), ("Param1", "value1")],
            extra_headers: &[],
            payload: &PayloadHash::Sha256Hex(EMPTY_SHA.into()),
            date: "20150830",
            datetime: "20150830T123600Z",
        });
        assert_eq!(
            r.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    /// Streaming mode: the sentinel header is emitted (S3 rule) and the
    /// signature stays deterministic per (request, datetime).
    #[test]
    fn unsigned_payload_mode_is_deterministic() {
        let signer = s3_signer();
        let a = signer.sign(SigningInput {
            method: "PUT",
            host: "127.0.0.1:9000",
            canonical_uri: "/ium-test/obj",
            query: &[("partNumber", "1"), ("uploadId", "deadbeef")],
            extra_headers: &[],
            payload: &PayloadHash::Unsigned,
            date: "20150830",
            datetime: "20150830T123600Z",
        });
        assert_eq!(a.x_amz_content_sha256, "UNSIGNED-PAYLOAD");
        let b = signer.sign(SigningInput {
            method: "PUT",
            host: "127.0.0.1:9000",
            canonical_uri: "/ium-test/obj",
            query: &[("partNumber", "1"), ("uploadId", "deadbeef")],
            extra_headers: &[],
            payload: &PayloadHash::Unsigned,
            date: "20150830",
            datetime: "20150830T123600Z",
        });
        assert_eq!(
            a.authorization, b.authorization,
            "same inputs, same signature"
        );

        let c = signer.sign(SigningInput {
            method: "PUT",
            host: "127.0.0.1:9000",
            canonical_uri: "/ium-test/obj",
            query: &[("partNumber", "1"), ("uploadId", "deadbeef")],
            extra_headers: &[],
            payload: &PayloadHash::Unsigned,
            date: "20150831",
            datetime: "20150831T123600Z",
        });
        assert_ne!(
            a.authorization, c.authorization,
            "different datetime, different scope"
        );
        assert!(c.authorization.contains("/20150831/"));
    }

    /// The signing-key chain is deterministic and date-scoped.
    #[test]
    fn derived_key_is_deterministic_and_date_scoped() {
        let k1 = derived_signing_key(SK_S3, "20130524", "us-east-1", "s3");
        let k2 = derived_signing_key(SK_S3, "20130524", "us-east-1", "s3");
        let k3 = derived_signing_key(SK_S3, "20130525", "us-east-1", "s3");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3, "scope date changes the key");
    }

    /// The helper must agree with Vector C's published payload hash.
    #[test]
    fn sha256_helper_matches_published_vector() {
        assert_eq!(
            sha256_hex(b"Welcome to Amazon S3."),
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
    }

    /// Query canonicalization: strict RFC3986, uppercase hex, `=` inside a
    /// value must be encoded.
    #[test]
    fn uri_encode_is_strict() {
        assert_eq!(uri_encode("aB9-._~"), "aB9-._~");
        assert_eq!(uri_encode("a b"), "a%20b");
        assert_eq!(uri_encode("a=b&c"), "a%3Db%26c");
        assert_eq!(uri_encode("ü"), "%C3%BC");
    }
}
