#!/usr/bin/env python3
"""Independent SigV4 presigned-URL reference (pure stdlib) — the same
triple-source discipline the header-signing vectors used in #6: the values
this prints get pinned byte-exact into ium-core/tests/sigv4_presign.rs.
Written fresh from the AWS SigV4 presigned-URL rules, NOT translated from
the Rust module.

Anchoring: every run FIRST re-derives the published AWS S3 developer-guide
Vector B header-form signature (GET /test.txt with a `range` header — the
same vector pinned in ium-core/src/sigv4.rs since #6) from THIS script's
own canonicalization + HMAC chain. If the anchor drifts, the script refuses
to mint presign values (exit 1) — so the pinned presign vectors are never
orphaned from the published AWS truth. The access key below is AWS's
PUBLIC documentation example key (AKIAIOSFODNN7EXAMPLE), not a credential.
"""
import hashlib
import hmac
import sys
import urllib.parse

AK = "AKIAIOSFODNN7EXAMPLE"
SK = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
REGION = "us-east-1"
SERVICE = "s3"
EMPTY_SHA = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"


def uri_encode(s: str) -> str:
    return urllib.parse.quote(s, safe="-._~")


def h(key: bytes, msg: str) -> bytes:
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def signing_key(date: str) -> bytes:
    k_date = h(f"AWS4{SK}".encode(), date)
    k_region = h(k_date, REGION)
    k_service = h(k_region, SERVICE)
    return h(k_service, "aws4_request")


def presign(host, canonical_uri, expires, date, datetime):
    scope = f"{date}/{REGION}/{SERVICE}/aws4_request"
    cred = f"{AK}/{scope}"
    # canonical query: sorted, URI-encoded key=value pairs
    pairs = [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
        ("X-Amz-Credential", cred),
        ("X-Amz-Date", datetime),
        ("X-Amz-Expires", str(expires)),
        ("X-Amz-SignedHeaders", "host"),
    ]
    pairs.sort(key=lambda kv: (uri_encode(kv[0]), uri_encode(kv[1])))
    canonical_query = "&".join(f"{uri_encode(k)}={uri_encode(v)}" for k, v in pairs)
    canonical_headers = f"host:{host}\n"
    signed_headers = "host"
    creq = f"GET\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\nUNSIGNED-PAYLOAD"
    sts = f"AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{hashlib.sha256(creq.encode()).hexdigest()}"
    sig = hmac.new(signing_key(date), sts.encode(), hashlib.sha256).hexdigest()
    return canonical_query + f"&X-Amz-Signature={sig}"


def vector_b_anchor() -> None:
    """Re-derive the PUBLISHED Vector B signature with THIS script's chain.

    AWS S3 developer guide, "Example: GET Object" with a range header:
    header-form signing over host;range;x-amz-content-sha256;x-amz-date,
    payload = the empty-body hash. The expected value is the one pinned in
    ium-core/src/sigv4.rs (`vector_s3_get_object_with_range`).
    """
    datetime = "20130524T000000Z"
    date = "20130524"
    scope = f"{date}/{REGION}/{SERVICE}/aws4_request"
    canonical_headers = (
        f"host:examplebucket.s3.amazonaws.com\n"
        f"range:bytes=0-9\n"
        f"x-amz-content-sha256:{EMPTY_SHA}\n"
        f"x-amz-date:{datetime}\n"
    )
    signed_headers = "host;range;x-amz-content-sha256;x-amz-date"
    creq = (
        f"GET\n/test.txt\n\n{canonical_headers}\n{signed_headers}\n{EMPTY_SHA}"
    )
    sts = f"AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{hashlib.sha256(creq.encode()).hexdigest()}"
    sig = hmac.new(signing_key(date), sts.encode(), hashlib.sha256).hexdigest()
    expected = (
        "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    )
    if sig != expected:
        print(f"ANCHOR DRIFT: derived {sig}, published {expected}", file=sys.stderr)
        sys.exit(1)
    print(f"vector-b anchor OK ({sig[:8]}…{sig[-6:]})")


V1 = dict(
    host="examplebucket.s3.amazonaws.com",
    canonical_uri="/test.txt",
    expires=86400,
    date="20130524",
    datetime="20130524T000000Z",
)
V2 = dict(
    host="127.0.0.1:9000",
    canonical_uri="/ium-test/obj%24name",
    expires=3600,
    date="20261111",
    datetime="20261111T120000Z",
)

vector_b_anchor()
print("V1 =", presign(**V1))
print("V2 =", presign(**V2))
