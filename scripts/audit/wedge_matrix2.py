#!/usr/bin/env python3
"""The #98 wedge matrix, round 2 — isolate the DELAY variable with identical
raw-socket mechanics, plus the original http.client cut as a control:

  D  8 MiB, raw cut at 2 MiB, FIRST re-PUT after 32 s   (delay isolated)
  E  8 MiB, http.client cut at 2 MiB (the ORIGINAL poison shape), 32 s
  F  8 MiB, raw cut at 2 MiB, re-PUTs at +3s / +33s / +63s  (persistence)

Exit 0 always (evidence, not gate).
"""
import http.client
import os
import socket
import time
import hashlib
import hmac
import datetime
import re
from urllib.parse import urlparse

ENDPOINT = os.environ["IUM_S3_ENDPOINT"]
AK = os.environ["IUM_S3_ACCESS_KEY"]
SK = os.environ["IUM_S3_SECRET_KEY"]
u = urlparse(ENDPOINT)
HOST, PORT = u.hostname, u.port or 80
BUCKET = u.path.strip("/")
PART = 8 * 1024 * 1024
CUT = 2 * 1024 * 1024


def sigv4(method, path, query, payload):
    t = datetime.datetime.now(datetime.timezone.utc)
    amz = t.strftime("%Y%m%dT%H%M%SZ")
    ds = t.strftime("%Y%m%d")
    region = "us-east-1"
    payload_hash = hashlib.sha256(payload).hexdigest()
    host = f"{HOST}:{PORT}"
    pairs = sorted((k, v) for k, v in query.items())
    q = "&".join(f"{k}={v}" for k, v in pairs)
    hdrs = {"host": host, "x-amz-content-sha256": payload_hash, "x-amz-date": amz}
    names = sorted(hdrs)
    chdrs = "".join(f"{n}:{hdrs[n]}\n" for n in names)
    creq = "\n".join([method, path, q, chdrs, ";".join(names), payload_hash])
    scope = f"{ds}/{region}/s3/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{amz}\n{scope}\n{hashlib.sha256(creq.encode()).hexdigest()}"
    k = hmac.new(("AWS4" + SK).encode(), ds.encode(), hashlib.sha256).digest()
    for s in (region, "s3", "aws4_request"):
        k = hmac.new(k, s.encode(), hashlib.sha256).digest()
    sig = hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()
    return {
        "Host": host,
        "x-amz-date": amz,
        "x-amz-content-sha256": payload_hash,
        "Authorization": (
            f"AWS4-HMAC-SHA256 Credential={AK}/{scope}, "
            f"SignedHeaders={';'.join(names)}, Signature={sig}"
        ),
    }


def req(method, path, query=None, body=b"", timeout=300):
    query = query or {}
    qs = "&".join(f"{k}={v}" for k, v in sorted(query.items()))
    full = f"{path}?{qs}" if qs else path
    h = sigv4(method, path, query, body)
    conn = http.client.HTTPConnection(HOST, PORT, timeout=timeout)
    conn.request(method, full, body=body, headers=h)
    r = conn.getresponse()
    data = r.read(4096)
    conn.close()
    return r.status, data


def cut_raw(path, query, total, cut_after):
    query_str = "&".join(f"{k}={v}" for k, v in sorted(query.items()))
    h = sigv4("PUT", path, query, b"\x00" * total)
    head = (
        f"PUT {path}?{query_str} HTTP/1.1\r\n"
        + "".join(f"{k}: {v}\r\n" for k, v in h.items())
        + f"Content-Length: {total}\r\n\r\n"
    )
    s = socket.create_connection((HOST, PORT), timeout=60)
    s.sendall(head.encode())
    sent = 0
    chunk = b"\x00" * (64 * 1024)
    try:
        while sent < cut_after:
            sent += s.send(chunk[: min(64 * 1024, cut_after - sent)])
    except OSError:
        pass
    s.close()
    return sent


def cut_httpclient(path, query, total, cut_after):
    """The ORIGINAL poison_experiment mechanics (http.client, 64 KiB chunks)."""
    body = bytes(64 * 1024)
    query_str = "&".join(f"{k}={v}" for k, v in sorted(query.items()))
    h = sigv4("PUT", path, query, b"\x00" * total)
    conn = http.client.HTTPConnection(HOST, PORT, timeout=60)
    conn.putrequest("PUT", f"{path}?{query_str}")
    for k, v in h.items():
        conn.putheader(k, v)
    conn.putheader("Content-Length", str(total))
    conn.endheaders()
    sent = 0
    while sent < cut_after:
        sent += conn.send(body[: min(64 * 1024, cut_after - sent)])
    conn.close()
    return sent


def leg(name, cut_fn, delays):
    st, data = req("POST", f"/{BUCKET}/{name}.bin", {"uploads": ""})
    uid = re.search(rb"<UploadId>([^<]+)</UploadId>", data).group(1).decode()
    moved = cut_fn(f"/{BUCKET}/{name}.bin", {"partNumber": "1", "uploadId": uid}, PART, CUT)
    print(f"[{name}] initiate {st}, cut at {moved}B")
    for d in delays:
        time.sleep(d)
        st, data = req("PUT", f"/{BUCKET}/{name}.bin",
                       {"partNumber": "1", "uploadId": uid}, b"\x02" * PART)
        tag = f"re-PUT +{d}s → {st}"
        if st != 200:
            tag += b" " + data[:80]
        print(f"[{name}] {tag}")
    st, _ = req("DELETE", f"/{BUCKET}/{name}.bin", {"uploadId": uid})
    print(f"[{name}] abort {st}")


def main():
    leg("D-raw-32s", cut_raw, [32])
    leg("E-hc-32s", cut_httpclient, [32])
    leg("F-raw-3-33", cut_raw, [3, 30, 30])


if __name__ == "__main__":
    main()
