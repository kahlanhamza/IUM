#!/usr/bin/env python3
"""The #98 wedge matrix — which exact condition wedges a SeaweedFS 4.51
multipart session? The drill evidence: after freeze+kill, every re-PUT of
the frozen part got 500 InternalError (attempts 7/8/9), while the poison
experiment's re-PUT (8 MiB part, cut, NO completed sibling, NO proxy) got
200. Three discriminating legs against one live weed (idleTimeout=30):

  A  8 MiB part, cut mid-body at 2 MiB            (poison shape, isolated)
  B  128 MiB part, cut mid-body at ~7 MiB          (drill part size)
  C  128 MiB completed part 1, then 128 MiB part 2 cut at ~7 MiB  (drill shape)

Each leg: initiate → (optional part1) → cut PUT → 3 s → re-PUT same part
→ re-PUT again after 30 s → verdict line. Exit 0 always (evidence, not gate).
"""
import http.client
import os
import time
import hashlib
import hmac
import datetime
import re
import sys
from urllib.parse import urlparse

ENDPOINT = os.environ["IUM_S3_ENDPOINT"]
AK = os.environ["IUM_S3_ACCESS_KEY"]
SK = os.environ["IUM_S3_SECRET_KEY"]
u = urlparse(ENDPOINT)
HOST, PORT = u.hostname, u.port or 80
BUCKET = u.path.strip("/")
PART_SMALL = 8 * 1024 * 1024
PART_BIG = 128 * 1024 * 1024
CUT_SMALL = 2 * 1024 * 1024
CUT_BIG = 7 * 1024 * 1024


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


def cut_put(path, query, total, cut_after):
    """Open the PUT and kill the socket mid-body (the freeze's server face).

    Raw socket: http.client's sendall would raise on the server-side reset
    and hide whatever the server said first. A reset DURING the send loop is
    itself evidence — the post-mortem recv grabs any early error body.
    """
    import socket
    query_str = "&".join(f"{k}={v}" for k, v in sorted(query.items()))
    body = b"\x00" * (64 * 1024)
    h = sigv4("PUT", path, query, b"\x00" * total)
    head = (
        f"PUT {path}?{query_str} HTTP/1.1\r\n"
        + "".join(f"{k}: {v}\r\n" for k, v in h.items())
        + f"Content-Length: {total}\r\n\r\n"
    )
    s = socket.create_connection((HOST, PORT), timeout=60)
    s.sendall(head.encode())
    sent = 0
    err = ""
    try:
        while sent < cut_after:
            n = s.send(body[: min(64 * 1024, cut_after - sent)])
            sent += n
    except OSError as e:
        err = f" (reset during send after {sent}B: {e})"
    try:
        s.settimeout(2)
        trickle = s.recv(512)
        if trickle:
            err += f" server-said: {trickle[:200]!r}"
    except OSError:
        pass
    s.close()
    print(f"    cut-socket closed at {sent}B{err}")
    return sent


def leg(name, big, sibling):
    part = PART_BIG if big else PART_SMALL
    cut = CUT_BIG if big else CUT_SMALL
    st, data = req("POST", f"/{BUCKET}/{name}.bin", {"uploads": ""})
    uid = re.search(rb"<UploadId>([^<]+)</UploadId>", data).group(1).decode()
    print(f"[{name}] initiate {st} uid={uid[:16]}…")
    if sibling:
        st, _ = req("PUT", f"/{BUCKET}/{name}.bin",
                    {"partNumber": "1", "uploadId": uid}, b"\x01" * part)
        print(f"[{name}] part1 completed {st}")
    moved = cut_put(f"/{BUCKET}/{name}.bin",
                    {"partNumber": "2" if sibling else "1", "uploadId": uid},
                    part, cut)
    print(f"[{name}] cut mid-body after {moved} bytes")
    time.sleep(3)
    st, data = req("PUT", f"/{BUCKET}/{name}.bin",
                   {"partNumber": "2" if sibling else "1", "uploadId": uid},
                   b"\x02" * part)
    print(f"[{name}] re-PUT +3s → {st} {data[:120] if st != 200 else b''}")
    time.sleep(30)
    st, data = req("PUT", f"/{BUCKET}/{name}.bin",
                   {"partNumber": "2" if sibling else "1", "uploadId": uid},
                   b"\x02" * part)
    print(f"[{name}] re-PUT +30s → {st} {data[:120] if st != 200 else b''}")
    st, _ = req("DELETE", f"/{BUCKET}/{name}.bin", {"uploadId": uid})
    print(f"[{name}] abort {st}")


def main():
    leg("A-small", big=False, sibling=False)
    leg("B-big-nosib", big=True, sibling=False)
    leg("C-big-sib", big=True, sibling=True)


if __name__ == "__main__":
    main()
