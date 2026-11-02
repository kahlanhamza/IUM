#!/usr/bin/env python3
"""Minimal SigV4-signed S3 client for the test harness (stdlib only).

Reuses the signing logic of the independent Python reference (tmp-ref/sigv4_ref.py,
written from the AWS spec for issue #6) so the harness carries no server-specific
CLI dependency: bucket creation, readiness probes and multipart-abort cleanup work
against any S3-compatible server (Garage, SeaweedFS, MinIO, real S3).

Usage:
  s3curl.py METHOD http://127.0.0.1:8333/ium-test [/key] [--file F | --data D]
            [--ak K --sk S --region R]
Prints the HTTP status code, then the response body. Exit 1 on network error.
"""
import argparse
import datetime
import hashlib
import hmac
import http.client
import re
import sys
import urllib.parse


def sha256_hex(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def hmac_bytes(key: bytes, msg: str) -> bytes:
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def derived_key(secret, date, region, service):
    k = hmac_bytes(("AWS4" + secret).encode(), date)
    k = hmac_bytes(k, region)
    k = hmac_bytes(k, service)
    return hmac_bytes(k, "aws4_request")


def sign_request(method, host, path, query, payload_hash, ak, sk, region, now):
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    date = now.strftime("%Y%m%d")
    # canonical query: sorted, urlencoded (safe='' encodes everything S3 needs)
    q = "&".join(
        f"{urllib.parse.quote(k, safe='')}={urllib.parse.quote(v, safe='')}"
        for k, v in sorted(query.items())
    )
    headers = {
        "host": host,
        "x-amz-content-sha256": payload_hash,  # S3 rule: required + signed
        "x-amz-date": amz_date,
    }
    names = sorted(headers)
    chdrs = "".join(f"{n}:{headers[n]}\n" for n in names)
    signed = ";".join(names)
    creq = f"{method}\n{path}\n{q}\n{chdrs}\n{signed}\n{payload_hash}"
    scope = f"{date}/{region}/s3/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{sha256_hex(creq.encode())}"
    sig = hmac.new(
        derived_key(sk, date, region, "s3"), sts.encode(), hashlib.sha256
    ).hexdigest()
    return {
        "Authorization": (
            f"AWS4-HMAC-SHA256 Credential={ak}/{scope}, "
            f"SignedHeaders={signed}, Signature={sig}"
        ),
        "x-amz-date": amz_date,
        "x-amz-content-sha256": payload_hash,
    }


def request(method, url, ak, sk, region, query=None, body=b""):
    """One signed request → (status, body bytes)."""
    u = urllib.parse.urlparse(url)
    path = u.path or "/"
    query = query or {}
    hdrs = sign_request(
        method.upper(), u.netloc, path, query, sha256_hex(body),
        ak, sk, region, datetime.datetime.now(datetime.timezone.utc),
    )
    if query:
        path = f"{path}?{urllib.parse.urlencode(sorted(query.items()))}"
    conn = http.client.HTTPConnection(u.hostname, u.port, timeout=300)
    try:
        conn.request(method.upper(), path, body=body, headers=hdrs)
        resp = conn.getresponse()
        return resp.status, resp.read(), resp
    finally:
        conn.close()


def abort_all_multipart_uploads(bucket_url: str, ak: str, sk: str, region: str) -> int:
    """List every in-progress multipart upload on the bucket and abort it.

    The harness's exit hygiene (issue #9 AC): a killed drill or a failed run
    must not leave orphaned multipart sessions on the server.
    Returns the number of aborted uploads.
    """
    status, out, _ = request("GET", bucket_url, ak, sk, region,
                             query={"uploads": ""})
    if status != 200:
        print(f"abort-multiparts: ListMultipartUploads -> {status}", file=sys.stderr)
        return 0
    text = out.decode("utf-8", "replace")
    uploads = re.findall(
        r"<Upload>.*?<Key>(.*?)</Key>.*?<UploadId>(.*?)</UploadId>.*?</Upload>",
        text,
        re.DOTALL,
    )
    n = 0
    for key, upload_id in uploads:
        url = f"{bucket_url.rstrip('/')}/{urllib.parse.quote(key, safe='')}"
        st, _, _ = request("DELETE", url, ak, sk, region,
                           query={"uploadId": upload_id})
        print(f"abort-multiparts: {key} uploadId={upload_id[:12]}... -> {st}")
        if 200 <= st < 300:
            n += 1
    return n


def main() -> None:
    if len(sys.argv) > 1 and sys.argv[1] == "--abort-multiparts":
        # s3curl.py --abort-multiparts http://127.0.0.1:8333/ium-test [--ak .. --sk .. --region ..]
        rest = sys.argv[2:]
        ak = rest[rest.index("--ak") + 1] if "--ak" in rest else "iumcikey"
        sk = (rest[rest.index("--sk") + 1] if "--sk" in rest
              else "iumcisecret00000000000000000000000000000000")
        region = rest[rest.index("--region") + 1] if "--region" in rest else "us-east-1"
        url = rest[0]
        print(abort_all_multipart_uploads(url, ak, sk, region))
        return

    ap = argparse.ArgumentParser()
    ap.add_argument("method")
    ap.add_argument("url")
    ap.add_argument("--ak", default="iumcikey")
    ap.add_argument("--sk", default="iumcisecret00000000000000000000000000000000")
    ap.add_argument("--region", default="us-east-1")
    ap.add_argument("--file")
    ap.add_argument("--data")
    ap.add_argument("--query", action="append", default=[],
                    help="repeatable key=value query pair")
    ap.add_argument("--headers", action="store_true",
                    help="print response status line + headers before the body")
    args = ap.parse_args()

    query = {}
    for kv in args.query:
        k, _, v = kv.partition("=")
        query[k] = v
    body = b""
    if args.file:
        with open(args.file, "rb") as f:
            body = f.read()
    elif args.data is not None:
        body = args.data.encode()

    try:
        status, out, resp = request(
            args.method, args.url, args.ak, args.sk, args.region,
            query=query, body=body,
        )
    except OSError as e:
        print(f"network error: {e}", file=sys.stderr)
        sys.exit(1)
    if args.headers:
        print(f"HTTP {status} {resp.reason}")
        for k, v in resp.getheaders():
            print(f"{k}: {v}")
        print()
    else:
        print(status)
    sys.stdout.buffer.write(out)
    if out and not out.endswith(b"\n"):
        print()


if __name__ == "__main__":
    main()
