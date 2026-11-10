#!/usr/bin/env python3
"""#98 wedge matrix, round 4 — the drill's FULL session shape.

The drill's session carries: a #57 canary probe part + completed parts +
the frozen part. Legs (all raw-socket bodies, s3curl initiate/abort):

  L  canary + part1 complete + part2(128 MiB) cut at 7 MiB → re-PUT  (no proxy)
  M  the SAME shape, but the cut rides a python NAT-twin proxy with the
     drill's exact kill mechanics (shutdown Both at +2 s, refusals during)

Exit 0 always (evidence, not gate).
"""
import http.client
import os
import socket
import sys
import time
import hashlib
import hmac
import datetime
import re
import threading
from urllib.parse import urlparse

ENDPOINT = os.environ["IUM_S3_ENDPOINT"]
AK = os.environ["IUM_S3_ACCESS_KEY"]
SK = os.environ["IUM_S3_SECRET_KEY"]
u = urlparse(ENDPOINT)
HOST, PORT = u.hostname, u.port or 80
BUCKET = u.path.strip("/")
BIG = 128 * 1024 * 1024
CUT = 7 * 1024 * 1024
CANARY = 64 * 1024

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import s3curl

BASE = f"http://{HOST}:{PORT}/{BUCKET}"


def sigv4(method, path, query, payload_hash, host):
    t = datetime.datetime.now(datetime.timezone.utc)
    amz = t.strftime("%Y%m%dT%H%M%SZ")
    ds = t.strftime("%Y%m%d")
    region = "us-east-1"
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
    return (
        f"PUT {path}?{q} HTTP/1.1\r\n"
        f"Host: {host}\r\n"
        f"x-amz-date: {amz}\r\n"
        f"x-amz-content-sha256: {payload_hash}\r\n"
        f"Authorization: AWS4-HMAC-SHA256 Credential={AK}/{scope}, "
        f"SignedHeaders={';'.join(names)}, Signature={sig}\r\n"
    )


def raw_status(sock_request_bytes, body_stream=None, total=None):
    """Send head (+ optional body chunks) → (status, body-snippet)."""
    s = socket.create_connection((HOST, PORT), timeout=300)
    s.sendall(sock_request_bytes.encode() if isinstance(sock_request_bytes, str) else sock_request_bytes)
    if body_stream is not None:
        sent = 0
        while sent < total:
            chunk = body_stream[: min(1024 * 1024, total - sent)] if isinstance(body_stream, bytes) else None
            s.sendall(chunk)
            sent += len(chunk)
    s.settimeout(120)
    buf = b""
    while b"\r\n\r\n" not in buf:
        piece = s.recv(65536)
        if not piece:
            break
        buf += piece
    head, _, rest = buf.partition(b"\r\n\r\n")
    status = int(head.split(b" ")[1]) if b" " in head else 0
    s.close()
    return status, rest[:150]


def put_part_head(key, part, uid, total, payload_hash):
    return sigv4("PUT", f"/{BUCKET}/{key}", {"partNumber": str(part), "uploadId": uid},
                 payload_hash, f"{HOST}:{PORT}") + f"Content-Length: {total}\r\n\r\n"


def leg_L():
    st, body, _ = s3curl.request("POST", f"{BASE}/L.bin", AK, SK, "us-east-1", query={"uploads": ""})
    uid = re.search(rb"<UploadId>([^<]+)</UploadId>", body).group(1).decode()
    print(f"[L] initiate {st}")
    # canary probe part (#57): a small real UploadPart into the session
    st, _ = raw_status(put_part_head("L.bin", 1, uid, CANARY, hashlib.sha256(b"\x00" * CANARY).hexdigest()),
                       b"\x00" * CANARY, CANARY)
    print(f"[L] canary part1 {st}")
    # completed part 2 (128 MiB)
    st, _ = raw_status(put_part_head("L.bin", 2, uid, BIG, hashlib.sha256(b"\x00" * BIG).hexdigest()),
                       b"\x00" * BIG, BIG)
    print(f"[L] part2 complete {st}")
    # part 3: cut at 7 MiB
    s = socket.create_connection((HOST, PORT), timeout=60)
    s.sendall(put_part_head("L.bin", 3, uid, BIG, hashlib.sha256(b"\x00" * BIG).hexdigest()).encode())
    sent = 0
    chunk = b"\x00" * (64 * 1024)
    try:
        while sent < CUT:
            sent += s.send(chunk[: min(64 * 1024, CUT - sent)])
    except OSError:
        pass
    s.close()
    print(f"[L] part3 cut at {sent}B; sleeping 3s")
    time.sleep(3)
    st, data = raw_status(put_part_head("L.bin", 3, uid, BIG, hashlib.sha256(b"\x00" * BIG).hexdigest()),
                          b"\x00" * BIG, BIG)
    print(f"[L] part3 re-PUT → {st} {data if st != 200 else ''}")
    s3curl.request("DELETE", f"{BASE}/L.bin", AK, SK, "us-east-1", query={"uploadId": uid})


# --- the NAT-twin (python twin of the drill's Rust proxy) -------------------

class Nat:
    def __init__(self):
        self.frozen = False
        self.conns = []
        self.lock = threading.Lock()

    def kill_all(self):
        self.frozen = True
        with self.lock:
            for a, b in self.conns:
                for sock in (a, b):
                    try:
                        sock.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass
            self.conns.clear()

    def thaw(self):
        self.frozen = False


def pump(src, dst, nat):
    try:
        while True:
            if nat.frozen:
                return
            data = src.recv(64 * 1024)
            if not data:
                try:
                    dst.shutdown(socket.SHUT_WR)
                except OSError:
                    pass
                return
            dst.sendall(data)
    except OSError:
        pass


def leg_M():
    nat = Nat()
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(16)
    proxy_port = listener.getsockname()[1]

    def accept_loop():
        while True:
            try:
                client, _ = listener.accept()
            except OSError:
                return
            if nat.frozen:
                client.close()
                continue
            server = socket.create_connection((HOST, PORT), timeout=60)
            with nat.lock:
                nat.conns.append((client, server))
            # exactly ONE pump per direction: c2s and s2c (a duplicate c2s
            # reader would split the client's bytes between two threads)
            threading.Thread(target=pump, args=(client, server, nat), daemon=True).start()
            threading.Thread(target=pump, args=(server, client, nat), daemon=True).start()

    threading.Thread(target=accept_loop, daemon=True).start()
    PHOST = f"127.0.0.1:{proxy_port}"

    def proxy_raw(head, body=None, total=None):
        s = socket.create_connection(("127.0.0.1", proxy_port), timeout=300)
        s.sendall(head.encode() if isinstance(head, str) else head)
        if body is not None:
            sent = 0
            while sent < total:
                s.sendall(body[: min(1024 * 1024, total - sent)])
                sent += min(1024 * 1024, total - sent)
        s.settimeout(120)
        buf = b""
        while b"\r\n\r\n" not in buf:
            piece = s.recv(65536)
            if not piece:
                break
            buf += piece
        head_r, _, rest = buf.partition(b"\r\n\r\n")
        status = int(head_r.split(b" ")[1]) if b" " in head_r else 0
        s.close()
        return status, rest[:150]

    st, body, _ = s3curl.request("POST", f"http://{PHOST}/{BUCKET}/M.bin", AK, SK, "us-east-1", query={"uploads": ""})
    uid = re.search(rb"<UploadId>([^<]+)</UploadId>", body).group(1).decode()
    print(f"[M] initiate via proxy {st}")
    st, _ = proxy_raw(put_part_head("M.bin", 1, uid, CANARY, hashlib.sha256(b"\x00" * CANARY).hexdigest()),
                      b"\x00" * CANARY, CANARY)
    print(f"[M] canary part1 {st}")
    st, _ = proxy_raw(put_part_head("M.bin", 2, uid, BIG, hashlib.sha256(b"\x00" * BIG).hexdigest()),
                      b"\x00" * BIG, BIG)
    print(f"[M] part2 complete {st}")
    # part 3 through the proxy, killed by kill_all at +2 s (the drill's exact kill)
    h = put_part_head("M.bin", 3, uid, BIG, hashlib.sha256(b"\x00" * BIG).hexdigest())
    s = socket.create_connection(("127.0.0.1", proxy_port), timeout=60)
    s.sendall(h.encode())
    sent = 0
    chunk = b"\x00" * (64 * 1024)
    while sent < CUT:
        sent += s.send(chunk[: min(64 * 1024, CUT - sent)])
    print(f"[M] part3 at {sent}B — freeze begins (proxy kill at +2s)")
    time.sleep(2)
    nat.kill_all()
    print("[M] NAT mapping expired")
    time.sleep(3)
    nat.thaw()
    st, data = proxy_raw(put_part_head("M.bin", 3, uid, BIG, hashlib.sha256(b"\x00" * BIG).hexdigest()),
                         b"\x00" * BIG, BIG)
    print(f"[M] part3 re-PUT → {st} {data if st != 200 else ''}")
    s3curl.request("DELETE", f"{BASE}/M.bin", AK, SK, "us-east-1", query={"uploadId": uid})


def main():
    leg_L()
    leg_M()


if __name__ == "__main__":
    main()
