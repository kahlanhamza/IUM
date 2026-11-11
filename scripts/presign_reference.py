#!/usr/bin/env python3
"""Independent SigV4 presigned-URL reference (pure stdlib) — the same
triple-source discipline the header-signing vectors used in #6: the values
this prints get pinned byte-exact into ium-core/tests/sigv4_presign.rs.
Written fresh from the AWS SigV4 presigned-URL rules, NOT translated from
the Rust module."""
import hashlib
import hmac
import urllib.parse

AK = "[REDACTED:aws_access_key]"
SK = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
REGION = "us-east-1"
SERVICE = "s3"


def uri_encode(s: str) -> str:
    return urllib.parse.quote(s, safe="-._~")


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

    def h(key: bytes, msg: str) -> bytes:
        return hmac.new(key, msg.encode(), hashlib.sha256).digest()

    k_date = h(f"AWS4{SK}".encode(), date)
    k_region = h(k_date, REGION)
    k_service = h(k_region, SERVICE)
    k_signing = h(k_service, "aws4_request")
    sig = hmac.new(k_signing, sts.encode(), hashlib.sha256).hexdigest()
    return canonical_query + f"&X-Amz-Signature={sig}"


V1 = dict(
    host="examplebucket.s3.amazonaws.com",
    canonical_uri="/test.txt",
    expires=86400,
    date="20130524",
    datetime="20130524T000000Z",
)
print("V1 =", presign(**V1))
V2 = dict(
    host="127.0.0.1:9000",
    canonical_uri="/ium-test/obj%24name",
    expires=3600,
    date="20261111",
    datetime="20261111T120000Z",
)
print("V2 =", presign(**V2))
