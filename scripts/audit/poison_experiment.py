#!/usr/bin/env python3
"""The #98 follow-up experiment: what does a mid-body-killed multipart PUT
poison in SeaweedFS 4.51? Sequence against a live weed (idleTimeout=30):

  1. initiate multipart
  2. PUT part 1 and CUT the connection mid-body (the freeze's server face)
  3. wait out the idle timeout
  4. re-PUT part 1 (full, fresh connection)  <- the engine's retry
  5. ListParts                                <- is the session alive?
  6. re-PUT part 1 again after a ListParts    <- does listing unstick it?

Prints one line per step: STEP-N <status>. Exit 0 always (evidence, not gate).
Env: IUM_S3_ENDPOINT, IUM_S3_ACCESS_KEY, IUM_S3_SECRET_KEY.
"""
import http.client
import os
import sys
import time
import hashlib
from urllib.parse import urlparse

ENDPOINT = os.environ["IUM_S3_ENDPOINT"]
AK = os.environ["IUM_S3_ACCESS_KEY"]
SK = os.environ["IUM_S3_SECRET_KEY"]
u = urlparse(ENDPOINT)
HOST, PORT = u.hostname, u.port or 80
BUCKET = u.path.strip("/")
PART_SIZE = 8 * 1024 * 1024


def sigv4_headers(method, path, headers=None, payload=b""):
    """Minimal SigV4 (the drill needs no SDK — same as the engine's signer).

    The canonical request is SIX lines: method, canonical URI (NO query),
    canonical query (sorted `k=v`, `&`-joined), canonical headers, signed
    header names, payload hash. The first draft merged `?uploads` into the
    URI and had no query line — the server computed a different creq and
    answered 403 (the RED-session wire fact; the engine's own signer does
    the six-line shape, this script now matches it).
    """
    import datetime
    t = datetime.datetime.now(datetime.timezone.utc)
    amzdate = t.strftime("%Y%m%dT%H%M%SZ")
    datestamp = t.strftime("%Y%m%d")
    region = os.environ.get("IUM_S3_REGION", "us-east-1")
    service = "s3"
    headers = dict(headers or {})
    headers["x-amz-date"] = amzdate
    headers["x-amz-content-sha256"] = hashlib.sha256(payload).hexdigest()
    signed = {k: v for k, v in headers.items() if k in
              ("host", "x-amz-date", "x-amz-content-sha256")}
    host = f"{HOST}:{PORT}"
    signed["Host"] = host
    canonical_headers = "".join(f"{k.lower()}:{signed[k].strip()}\n"
                                for k in sorted(signed, key=str.lower))
    # SignedHeaders names must be LOWERCASE — `sorted(key=str.lower)` sorts
    # correctly but preserves the original case, and `Host;…` (capital H)
    # signed the wrong canonical request (the 403 body said it outright).
    signed_names = ";".join(sorted(k.lower() for k in signed))
    # Split URI and query (SigV4 line 2 vs line 3). Query params: sorted,
    # `k=v` even when the value is empty (`uploads=`).
    if "?" in path:
        uri, qs = path.split("?", 1)
        pairs = []
        for p in qs.split("&"):
            if not p:
                continue
            if "=" in p:
                k, v = p.split("=", 1)
            else:
                k, v = p, ""
            pairs.append((k, v))
        pairs.sort()
        canonical_query = "&".join(f"{k}={v}" for k, v in pairs)
    else:
        uri, canonical_query = path, ""
    creq = "\n".join([method, uri, canonical_query, canonical_headers,
                      signed_names, headers["x-amz-content-sha256"]])
    import hmac
    scope = f"{datestamp}/{region}/{service}/aws4_request"
    def _h(b):
        return hashlib.sha256(b).hexdigest()
    sts = f"AWS4-HMAC-SHA256\n{amzdate}\n{scope}\n{_h(creq.encode())}"
    k = hmac.new(("AWS4" + SK).encode(), datestamp.encode(), hashlib.sha256).digest()
    k = hmac.new(k, region.encode(), hashlib.sha256).digest()
    k = hmac.new(k, service.encode(), hashlib.sha256).digest()
    k = hmac.new(k, b"aws4_request", hashlib.sha256).digest()
    sig = hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()
    headers["Authorization"] = (
        f"AWS4-HMAC-SHA256 Credential={AK}/{scope}, "
        f"SignedHeaders={signed_names}, Signature={sig}")
    return headers


def req(method, path, body=b"", headers=None, timeout=30):
    conn = http.client.HTTPConnection(HOST, PORT, timeout=timeout)
    h = sigv4_headers(method, path, headers, body)
    conn.request(method, path, body=body, headers=h)
    r = conn.getresponse()
    data = r.read(2048)
    conn.close()
    return r.status, data


def cut_put_part(upload_id, part_no, cut_after):
    """Open a PUT of a part and kill the socket mid-body (the freeze face)."""
    path = f"/{BUCKET}/poison.bin?partNumber={part_no}&uploadId={upload_id}"
    body = bytes(range(256)) * (PART_SIZE // 256)
    conn = http.client.HTTPConnection(HOST, PORT, timeout=60)
    h = sigv4_headers("PUT", path, None, body)
    h["Content-Length"] = str(len(body))
    conn.putrequest("PUT", path)
    for k, v in h.items():
        if k.lower() == "host":
            conn.putheader("Host", f"{HOST}:{PORT}")
        else:
            conn.putheader(k, v)
    conn.endheaders()
    sent = 0
    chunk = 64 * 1024
    for off in range(0, len(body), chunk):
        conn.send(body[off:off + chunk])
        sent = min(off + chunk, len(body))
        if sent >= cut_after:
            break
    conn.close()  # RST/FIN mid-body — the server's idle/parse loop eats it
    return sent


def main():
    part1 = bytes(range(256)) * (PART_SIZE // 256)
    status, _ = req("POST", f"/{BUCKET}/poison.bin?uploads")
    print("STEP-1 initiate", status)
    status, data = req("POST", f"/{BUCKET}/poison.bin?uploads")
    import re
    m = re.search(rb"<UploadId>([^<]+)</UploadId>", data)
    uid = m.group(1).decode()
    print("STEP-1b uploadId", uid[:24], "…")

    moved = cut_put_part(uid, 1, cut_after=2 * 1024 * 1024)
    print("STEP-2 cut mid-body after", moved, "bytes (socket closed)")

    print("STEP-3 waiting 32s for the idle killer…")
    time.sleep(32)

    st, data = req("PUT", f"/{BUCKET}/poison.bin?partNumber=1&uploadId={uid}", part1)
    print("STEP-4 re-put part1 (full)", st, data[:160])
    if st != 200:
        st, data = req("PUT", f"/{BUCKET}/poison.bin?partNumber=1&uploadId={uid}", part1)
        print("STEP-4b re-put AGAIN", st, data[:160])

    st, data = req("GET", f"/{BUCKET}/poison.bin?uploadId={uid}")
    print("STEP-5 ListParts", st, data[:200])

    st, data = req("PUT", f"/{BUCKET}/poison.bin?partNumber=2&uploadId={uid}", part1)
    print("STEP-6 put part2 (other number)", st, data[:160])

    st, data = req("PUT", f"/{BUCKET}/poison.bin?partNumber=1&uploadId={uid}", part1)
    print("STEP-7 re-put part1 again", st, data[:160])

    # cleanup: abort the session
    st, _ = req("DELETE", f"/{BUCKET}/poison.bin?uploadId={uid}")
    print("STEP-8 abort", st)


if __name__ == "__main__":
    main()
