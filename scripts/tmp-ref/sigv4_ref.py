#!/usr/bin/env python3
"""Independent SigV4 reference (Python stdlib) to verify vector constants
before they are pinned in the Rust test suite. Written from the AWS spec,
not from the Rust implementation. Scratch tool — not part of the build."""
import hashlib
import hmac


def sha256_hex(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def hmac_hex(key: bytes, msg: str) -> str:
    return hmac.new(key, msg.encode(), hashlib.sha256).hexdigest()


def hmac_bytes(key: bytes, msg: str) -> bytes:
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def derived_key_bytes(secret, date, region, service):
    k = hmac_bytes(("AWS4" + secret).encode(), date)
    k = hmac_bytes(k, region)
    k = hmac_bytes(k, service)
    return hmac_bytes(k, "aws4_request")


def sign(method, host, uri, query_pairs, extra_headers, payload_hash,
         date, datetime, ak, sk, region, service):
    q = "&".join(f"{k}={v}" for k, v in sorted(query_pairs))
    headers = {k.lower(): v.strip() for k, v in extra_headers}
    headers["host"] = host.strip()
    headers["x-amz-date"] = datetime
    # S3-specific rule: x-amz-content-sha256 is REQUIRED and SIGNED as a
    # header. Generic services only use the payload hash as the creq's last
    # line. (This is why get-vanilla signs host;x-amz-date only.)
    if service == "s3":
        headers["x-amz-content-sha256"] = payload_hash
    names = sorted(headers)
    chdrs = "".join(f"{n}:{headers[n]}\n" for n in names)
    signed = ";".join(names)
    creq = f"{method}\n{uri}\n{q}\n{chdrs}\n{signed}\n{payload_hash}"
    scope = f"{date}/{region}/{service}/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{sha256_hex(creq.encode())}"
    sig = hmac_hex(derived_key_bytes(sk, date, region, service), sts)
    auth = (f"AWS4-HMAC-SHA256 Credential={ak}/{scope}, "
            f"SignedHeaders={signed}, Signature={sig}")
    return creq, sts, auth


EMPTY = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

if __name__ == "__main__":
    print("kSigning(B):", derived_key_bytes(
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "20130524", "us-east-1", "s3").hex())
    print("sha256('Welcome to Amazon S3.'):", sha256_hex(b"Welcome to Amazon S3."))

    # A: get-vanilla
    _, _, a = sign("GET", "example.amazonaws.com", "/", [],
                   [], EMPTY, "20150830", "20150830T123600Z",
                   "AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                   "us-east-1", "service")
    print("A auth:", a)

    # B: S3 GET object (range header)
    _, _, b = sign("GET", "examplebucket.s3.amazonaws.com", "/test.txt", [],
                   [("range", "bytes=0-9")], EMPTY,
                   "20130524", "20130524T000000Z",
                   "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                   "us-east-1", "s3")
    print("B auth:", b)

    # C: S3 PUT object (date + storage-class headers, body hash)
    bh = sha256_hex(b"Welcome to Amazon S3.")
    _, _, c = sign("PUT", "examplebucket.s3.amazonaws.com", "/test%24file.text", [],
                   [("date", "Fri, 24 May 2013 00:00:00 GMT"),
                    ("x-amz-storage-class", "REDUCED_REDUNDANCY")], bh,
                   "20130524", "20130524T000000Z",
                   "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                   "us-east-1", "s3")
    print("C auth:", c)

    # D: single query param
    _, _, d = sign("GET", "example.amazonaws.com", "/",
                   [("Param1", "value1")], [], EMPTY,
                   "20150830", "20150830T123600Z",
                   "AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                   "us-east-1", "service")
    print("D auth:", d)

    # E: two query params, raw order reversed -> canonical sorted
    _, _, e = sign("GET", "example.amazonaws.com", "/",
                   [("Param2", "value2"), ("Param1", "value1")], [], EMPTY,
                   "20150830", "20150830T123600Z",
                   "AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                   "us-east-1", "service")
    print("E auth:", e)
