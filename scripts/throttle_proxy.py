#!/usr/bin/env python3
"""Transparent throttling TCP proxy for the real-world hostile-network drill
(issue #22, scenario S3).

Sits in front of the REAL S3 server; the production adapter talks THROUGH it.
The impairment lives in the network path — real TCP backpressure from paced
socket reads — not in a test double and not in a test clock.

Impairment model ("one bad Wi-Fi minute"): the FIRST connection ever to carry
an UploadPart for the victim partNumber is throttled to VICTIM_RATE bytes/s
(client -> server direction). The impairment binds to that ONE connection
until its body ends — `--victim-seconds` only windows the internal
victim_active_until bookkeeping, it does NOT un-throttle the link mid-body
(the engine is expected to abandon the impaired attempt long before the body
completes). Every later connection — including the eviction retry's fresh
connection — runs at full speed: the network healed, the engine is expected
to have abandoned the stuck lane and re-driven the span elsewhere.

Framing: the proxy forces `connection: close` on forwarded request heads, so
each connection carries exactly one request/response pair. That keeps the
parsing honest (one request head, one content-length body, one response head,
one content-length body) and gives the engine's connection pool a fresh
connection per request — the retry can never inherit the impaired socket.

Usage: throttle_proxy.py LISTEN_PORT TARGET_HOST TARGET_PORT
       [--victim-part 2] [--victim-rate 98304] [--victim-seconds 45]
"""
import argparse
import asyncio
import sys
import time

CHUNK = 64 * 1024
IDLE_TIMEOUT = 600


class Relay:
    """One client connection: exactly one request/response pair."""

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                 target: tuple[str, int], victim_part: int, victim_rate: int,
                 victim_seconds: int, state: dict):
        self.r = reader
        self.w = writer
        self.target = target
        self.victim_part = victim_part
        self.victim_rate = victim_rate
        self.victim_seconds = victim_seconds
        self.state = state  # shared: {"victim_active_until": float|None, "victim_taken": bool}

    async def read_head(self) -> tuple[str, dict[str, str], bytes] | None:
        """Read one request head: request line + headers, ending at CRLFCRLF."""
        head = bytearray()
        while b"\r\n\r\n" not in head:
            b = await self.r.read(CHUNK)
            if not b:
                return None
            head += b
            if len(head) > 64 * 1024:
                return None
        raw, _, rest = bytes(head).partition(b"\r\n\r\n")
        lines = raw.decode("latin-1").split("\r\n")
        request_line = lines[0]
        headers: dict[str, str] = {}
        for ln in lines[1:]:
            if ":" in ln:
                k, _, v = ln.partition(":")
                headers[k.strip().lower()] = v.strip()
        return request_line, headers, rest

    async def pump(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                   count: int, rate: int | None, pre: bytes = b"") -> bool:
        """Move exactly `count` body bytes; pace reads when a rate is set.

        `pre` carries body bytes ALREADY consumed from the wire (they arrived
        glued to the request head in one TCP read). They are written first and
        count toward the pacing budget — dropping them deadlocks both sides:
        the proxy then waits for bytes the client already sent, while the
        client waits for a response that can never complete.
        """
        moved = 0
        start = time.monotonic()
        if pre:
            n = min(len(pre), count)
            dst.write(pre[:n])
            await dst.drain()
            moved += n
        while moved < count:
            n = min(CHUNK, count - moved)
            b = await src.read(n)
            if not b:
                break
            dst.write(b)
            await dst.drain()
            moved += len(b)
            if rate:
                target_t = start + moved / rate
                behind = target_t - time.monotonic()
                if behind > 0:
                    await asyncio.sleep(behind)
        return moved == count  # False = client vanished before the full body

    async def pump_until_eof(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                             rate: int | None, pre: bytes = b"") -> None:
        moved = len(pre)
        start = time.monotonic()
        if pre:
            dst.write(pre)
            await dst.drain()
        while True:
            b = await src.read(CHUNK)
            if not b:
                return
            dst.write(b)
            await dst.drain()
            moved += len(b)
            if rate:
                target_t = start + moved / rate
                behind = target_t - time.monotonic()
                if behind > 0:
                    await asyncio.sleep(behind)

    async def run(self) -> None:
        try:
            head = await asyncio.wait_for(self.read_head(), timeout=IDLE_TIMEOUT)
        except asyncio.TimeoutError:
            head = None
        if head is None:
            self.w.close()
            return
        request_line, headers, rest = head
        method = request_line.split(" ")[0] if " " in request_line else ""
        path = request_line.split(" ")[1] if " " in request_line else ""
        length = int(headers.get("content-length", "0"))
        te = "chunked" in headers.get("transfer-encoding", "").lower()
        print(f"[throttle-proxy] req {request_line}", flush=True)

        # --- victim selection: first connection EVER carrying the victim part
        rate: int | None = None
        if (f"partNumber={self.victim_part}" in path and method == "PUT"
                and not self.state["victim_taken"]):
            self.state["victim_taken"] = True
            self.state["victim_active_until"] = time.monotonic() + self.victim_seconds
            rate = self.victim_rate
            print(f"[throttle-proxy] VICTIM: {request_line} -> {self.victim_rate} B/s "
                  f"(binds to this connection's body)", flush=True)
        elif self.state["victim_active_until"] and time.monotonic() < self.state["victim_active_until"]:
            # NOTE: any other request that starts inside the impairment window
            # still moves at full speed — the impairment is bound to the ONE
            # impaired link (this connection was chosen before this branch),
            # not to the whole network. Nothing to do here.
            pass

        # --- connect upstream
        try:
            ureader, uwriter = await asyncio.open_connection(*self.target)
        except OSError as e:
            print(f"[throttle-proxy] upstream connect failed: {e}", file=sys.stderr)
            self.w.close()
            return

        # --- forward the head, forcing connection: close
        lines = request_line + "\r\n"
        for k, v in headers.items():
            if k in ("connection", "proxy-connection", "keep-alive"):
                continue
            lines += f"{k}: {v}\r\n"
        lines += "connection: close\r\n\r\n"
        uwriter.write(lines.encode("latin-1"))
        await uwriter.drain()

        # --- forward the body (paced iff victim) ---------------------------
        # `rest` is the body's first slice, read together with the head in one
        # TCP read. It MUST go upstream first and count in the pacing budget.
        if te:
            await self.pump_until_eof(self.r, uwriter, rate, pre=rest)  # best effort
        elif length:
            body_complete = await self.pump(self.r, uwriter, length, rate, pre=rest)
            if not body_complete:
                # The client abandoned this connection mid-body — this is the
                # engine's eviction tearing the impaired attempt off the wire.
                # The server can never finish this request; relay nothing.
                # Close BOTH ends now so the upstream sees a real disconnect
                # instead of a proxy that quietly waits forever.
                print(f"[throttle-proxy] client abandoned {request_line.split(' ')[0]} "
                      f"{path} mid-body (partial of {length} B) — tearing the link down", flush=True)
                for wr in (uwriter, self.w):
                    try:
                        wr.close()
                    except OSError:
                        pass
                return

        # --- relay the response head + body back, then close both ends
        try:
            resp_head = bytearray()
            while b"\r\n\r\n" not in resp_head:
                b = await asyncio.wait_for(ureader.read(CHUNK), timeout=IDLE_TIMEOUT)
                if not b:
                    break
                resp_head += b
            raw, _, rest = bytes(resp_head).partition(b"\r\n\r\n")
            rlines = raw.decode("latin-1").split("\r\n")
            rheaders: dict[str, str] = {}
            for ln in rlines[1:]:
                if ":" in ln:
                    k, _, v = ln.partition(":")
                    rheaders[k.strip().lower()] = v.strip()
            self.w.write(bytes(resp_head))
            await self.w.drain()
            if method == "HEAD":
                pass  # a HEAD response declares a length but carries no body
            elif "content-length" in rheaders:
                clen = int(rheaders["content-length"])
                if clen > len(rest):
                    await self.pump(ureader, self.w, clen - len(rest), None)
            else:
                # chunked or close-delimited: the connection closes anyway
                await self.pump_until_eof(ureader, self.w, None)
        except (OSError, ValueError, asyncio.TimeoutError) as e:
            print(f"[throttle-proxy] response relay error: {e}", file=sys.stderr)
        finally:
            print(f"[throttle-proxy] done {method} {path}", flush=True)
            for wr in (uwriter, self.w):
                try:
                    wr.close()
                except OSError:
                    pass


async def main_async(a: argparse.Namespace) -> None:
    state = {"victim_taken": False, "victim_active_until": None}
    target = (a.target_host, a.target_port)

    async def on_conn(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        relay = Relay(reader, writer, target, a.victim_part, a.victim_rate,
                      a.victim_seconds, state)
        await relay.run()

    server = await asyncio.start_server(on_conn, "127.0.0.1", a.listen_port)
    print(f"[throttle-proxy] listening 127.0.0.1:{a.listen_port} -> {target[0]}:{target[1]} "
          f"(victim partNumber={a.victim_part} @ {a.victim_rate} B/s, {a.victim_seconds}s, once)", flush=True)
    async with server:
        await server.serve_forever()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("listen_port", type=int)
    ap.add_argument("target_host")
    ap.add_argument("target_port", type=int)
    ap.add_argument("--victim-part", type=int, default=2)
    ap.add_argument("--victim-rate", type=int, default=96 * 1024)
    ap.add_argument("--victim-seconds", type=int, default=45)
    a = ap.parse_args()
    try:
        asyncio.run(main_async(a))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
