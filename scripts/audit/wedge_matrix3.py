#!/usr/bin/env python3
"""#98 wedge matrix, round 3 — the LARGE-PART variable.

The drill maps one chunk = ONE S3 part (adapter_s3.rs:12), and its freeze
shape declares 128 MiB parts. Round-1 leg C hinted a complete 128 MiB PUT
can 500 on SeaweedFS 4.51. Legs (fresh weed, idleTimeout=30 unless said):

  G  complete 128 MiB PUT (no cut)            — can the server take one at all?
  H  128 MiB declared, cut at 7 MiB, re-PUT +3s   (early)
  I  128 MiB declared, cut at 7 MiB, re-PUT +32s  (late)

Re-PUTs stream via a file-like body with explicit Content-Length (the
gateway rejects length-less uploads — adapter_s3.rs:925 note).
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
BIG = 128 * 1024 * 1024
CUT = 7 * 1024 * 1024


def sigv4(method, path, query, payload_hash):
    t = datetime.datetime.now(datetime.timezone.utc)
    amz = t.strftime("%Y%m%dT%H%M%SZ")
    ds = t.strftime("%Y%m%d")
    region = "us-east-1"
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


def read_response(s):
    """Read status line + body (Content-Length-honoring) from a raw socket."""
    s.settimeout(120)
    buf = b""
    try:
        while b"\r\n\r\n" not in buf:
            piece = s.recv(4096)
            if not piece:
                break
            buf += piece
        head, _, rest = buf.partition(b"\r\n\r\n")
        status = int(head.split(b" ")[1]) if b" " in head else 0
        cl = 0
        for line in head.split(b"\r\n"):
            if line.lower().startswith(b"content-length:"):
                cl = int(line.split(b":")[1].strip())
        body = rest
        s.settimeout(10)
        while len(body) < cl:
            piece = s.recv(4096)
            if not piece:
                break
            body += piece
        return status, body[:300]
    except OSError as e:
        return 0, repr(e).encode()


def full_put(name, query, body_sha_hex, total, body_file, delay_desc=""):
    """PUT via a raw socket: one head, then 1 MiB body chunks. No header
    duplication (the http.client double-Content-Length killed the initiate)."""
    query_str = "&".join(f"{k}={v}" for k, v in sorted(query.items()))
    path = f"/{BUCKET}/{name}"
    h = sigv4("PUT", path, query, body_sha_hex)
    head = (
        f"PUT {path}?{query_str} HTTP/1.1\r\n"
        + "".join(f"{k}: {v}\r\n" for k, v in h.items())
        + f"Content-Length: {total}\r\n\r\n"
    )
    s = socket.create_connection((HOST, PORT), timeout=300)
    s.sendall(head.encode())
    sent = 0
    while True:
        chunk = body_file.read(1024 * 1024)
        if not chunk:
            break
        s.sendall(chunk)
        sent += len(chunk)
    st, data = read_response(s)
    s.close()
    return st, data


def cut_raw(name, query, total, cut_after):
    query_str = "&".join(f"{k}={v}" for k, v in sorted(query.items()))
    path = f"/{BUCKET}/{name}"
    h = sigv4("PUT", path, query, hashlib.sha256(b"\x00" * total).hexdigest())
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


class ZeroFile:
    """A read()-only zero stream of `total` bytes (1 MiB reads)."""

    def __init__(self, total):
        self.left = total

    def read(self, n=-1):
        if self.left <= 0:
            return b""
        n = self.left if n < 0 else min(n, self.left)
        self.left -= n
        return b"\x00" * n


def leg(name, with_cut, delay):
    import io
    import sys
    sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
    import s3curl
    base = f"http://{HOST}:{PORT}/{BUCKET}"
    st, body, _ = s3curl.request("POST", f"{base}/{name}", AK, SK, "us-east-1",
                                 query={"uploads": ""})
    uid = re.search(rb"<UploadId>([^<]+)</UploadId>", body).group(1).decode()
    print(f"[{name}] initiate {st}")
    if with_cut:
        moved = cut_raw(name, {"partNumber": "1", "uploadId": uid}, BIG, CUT)
        print(f"[{name}] cut at {moved}B; sleeping {delay}s")
        time.sleep(delay)
    st, data = full_put(name, {"partNumber": "1", "uploadId": uid},
                        hashlib.sha256(b"\x00" * BIG).hexdigest(), BIG, ZeroFile(BIG))
    print(f"[{name}] full {BIG//1024//1024}MiB re-PUT → {st}"
          + ("" if st == 200 else f" {data[:120]!r}"))
    st, _, _ = s3curl.request("DELETE", f"{base}/{name}", AK, SK, "us-east-1",
                              query={"uploadId": uid})
    print(f"[{name}] abort {st}")


def main():
    leg("G-complete", with_cut=False, delay=0)
    leg("H-cut-early", with_cut=True, delay=3)
    leg("I-cut-late", with_cut=True, delay=32)


if __name__ == "__main__":
    main()
