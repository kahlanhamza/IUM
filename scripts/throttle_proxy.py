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

UNIFORM mode (issue #28, `--victim-part all`): EVERY UploadPart PUT
connection is throttled to VICTIM_RATE bytes/s for its whole body — the
"the entire uplink is slow" model, the owner's B/s-class daily reality (the
whole link, not one bad peer). There is no latch and no expiry window: the
link is slow for the scenario's whole life, and the scenario ends when the
engine's own timebox cancels the job (tearing the paced bodies off the wire).
Every throttled connection logs a `VICTIM-WIRE moved=<N>B in <S>s (<R> B/s)`
line when its body phase ends. PLUS the issue #28 wire meter, which must NOT
depend on TCP teardown: a canceled engine's FIN queues BEHIND ~1 MB of unread
paced data (FIN-WAIT-1 until the proxy drains it — ~1 hour at 512 B/s), so
EOF/RST can never surface in a bounded test. Instead the proxy keeps a global
relayed-bytes counter and prints `VICTIM-TICK moved=<N>B` every ~10 s while
pacing, plus one `VICTIM-FINAL moved=<N>B` on SIGTERM — the harness sums
those and never waits for the socket to die.

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
import os
import signal
import sys
import time

CHUNK = 64 * 1024
IDLE_TIMEOUT = 600


class Relay:
    """One client connection: exactly one request/response pair."""

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                 target: tuple[str, int], victim_part: str, victim_rate: int,
                 victim_seconds: int, state: dict):
        self.r = reader
        self.w = writer
        self.target = target
        self.victim_part = victim_part
        self.uniform = victim_part == "all"  # issue #28: the WHOLE uplink is slow
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

    @staticmethod
    async def _read_capped(src: asyncio.StreamReader, n: int, timeout: float):
        """Read up to n bytes, giving up after `timeout` so the caller's loop
        stays free to check the pacing schedule and the client's FIN. Returns
        None on timeout (no information), b"" on real EOF, bytes otherwise.
        """
        try:
            return await asyncio.wait_for(src.read(n), timeout=timeout)
        except asyncio.TimeoutError:
            return None

    async def _paced_relay(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                           until: int | None, rate: int | None, pre: bytes) -> int:
        """Relay the request body client→server, paced to `rate` B/s when set.

        Cyclic on purpose (issue #28 lesson): a long unbroken pacing sleep at
        B/s-class rates hides the client's FIN for minutes — at 512 B/s a
        single 64 KB buffer burst is ~2 minutes of schedule debt, and a naive
        `sleep(debt)` would never notice the engine canceling the job. Every
        cycle: read (capped at 250 ms), relay, then advance the schedule in
        ≤250 ms steps — the schedule is target-based (`start + moved/rate`),
        so the steps never change the rate, they only keep us responsive.
        Returns the bytes actually moved; the caller logs them (VICTIM-WIRE).
        """
        moved = 0
        start = time.monotonic()
        last_tick = time.monotonic()
        if pre:
            n = min(len(pre), until) if until is not None else len(pre)
            dst.write(pre[:n])
            await dst.drain()
            moved += n
            if rate:
                self.state["wire_moved"] += n
        while until is None or moved < until:
            if rate:
                # Pay the FULL pacing debt BEFORE the next read — the schedule
                # is target-based (`start + moved/rate`), so this IS the exact
                # rate shape, and the un-read socket is exactly the intended
                # impairment: the engine's writes block on TCP backpressure.
                # The cost: with data sitting unread, `at_eof()` stays False
                # (it demands an empty buffer), so client death is watched
                # WITHOUT consuming — the transport stays registered between
                # paced reads, so the kernel's RST when the client process
                # exits lands in the reader as an exception (`exception()` is
                # public API and pollable); a drained clean FIN shows in
                # `at_eof()`. The issue #28 lesson: a blind `sleep(debt)` at
                # 512 B/s is ~2 minutes per 64 KB burst and hides everything.
                while True:
                    behind = start + moved / rate - time.monotonic()
                    if behind <= 0:
                        break
                    now = time.monotonic()
                    if now - last_tick >= 10.0:
                        last_tick = now
                        print(f"[throttle-proxy] VICTIM-TICK moved={self.state['wire_moved']}B",
                              flush=True)
                    if src.exception() is not None or src.at_eof():
                        return moved  # the client is gone — log what moved
                    await asyncio.sleep(0.25)
            n = CHUNK if until is None else min(CHUNK, until - moved)
            try:
                b = await asyncio.wait_for(src.read(n), timeout=0.25)
            except asyncio.TimeoutError:
                continue  # schedule is due but the wire is idle — re-check
            except (ConnectionResetError, OSError):
                return moved  # RST surfaced through the read itself
            if b == b"":
                return moved  # the client is gone (EOF at the transport)
            dst.write(b)
            await dst.drain()
            moved += len(b)
            if rate:
                self.state["wire_moved"] += len(b)
        return moved

    async def pump(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                   count: int, rate: int | None, pre: bytes = b"") -> int:
        """Move exactly `count` body bytes; pace reads when a rate is set.

        `pre` carries body bytes ALREADY consumed from the wire (they arrived
        glued to the request head in one TCP read). They are written first and
        count toward the pacing budget — dropping them deadlocks both sides:
        the proxy then waits for bytes the client already sent, while the
        client waits for a response that can never complete.
        """
        return await self._paced_relay(src, dst, count, rate, pre)

    async def pump_until_eof(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                             rate: int | None, pre: bytes = b"") -> int:
        return await self._paced_relay(src, dst, None, rate, pre)

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

        # --- victim selection -------------------------------------------------
        # uniform mode: every UploadPart PUT is a slow-link body, no latch, no
        # expiry — the scenario ends when the engine cancels (issue #28).
        # victim mode: the FIRST connection EVER carrying the victim part.
        rate: int | None = None
        is_part_put = "partNumber=" in path and method == "PUT"
        if self.uniform and is_part_put:
            rate = self.victim_rate
            print(f"[throttle-proxy] UNIFORM: {request_line} -> {self.victim_rate} B/s "
                  f"(binds to this connection's body)", flush=True)
        elif (not self.uniform) and (f"partNumber={self.victim_part}" in path and method == "PUT"
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
        t_body0 = time.monotonic()
        req_moved = 0
        if te:
            req_moved = await self.pump_until_eof(self.r, uwriter, rate, pre=rest)  # best effort
        elif length:
            req_moved = await self.pump(self.r, uwriter, length, rate, pre=rest)
            if req_moved != length:
                # The client abandoned this connection mid-body — this is the
                # engine's eviction/resplit/cancel tearing the impaired attempt
                # off the wire. The server can never finish this request; relay
                # nothing. Close BOTH ends now so the upstream sees a real
                # disconnect instead of a proxy that quietly waits forever.
                print(f"[throttle-proxy] client abandoned {request_line.split(' ')[0]} "
                      f"{path} mid-body (moved {req_moved} of {length} B) — tearing the link down", flush=True)
                if rate:
                    el = time.monotonic() - t_body0
                    print(f"[throttle-proxy] VICTIM-WIRE moved={req_moved}B in {el:.1f}s "
                          f"({req_moved / el:.0f} B/s) {method} {path}", flush=True)
                for wr in (uwriter, self.w):
                    try:
                        wr.close()
                    except OSError:
                        pass
                return
        if rate:
            el = time.monotonic() - t_body0
            print(f"[throttle-proxy] VICTIM-WIRE moved={req_moved}B in {el:.1f}s "
                  f"({req_moved / el:.0f} B/s) {method} {path}", flush=True)

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
    state = {"victim_taken": False, "victim_active_until": None, "wire_moved": 0}
    target = (a.target_host, a.target_port)

    async def on_conn(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        relay = Relay(reader, writer, target, a.victim_part, a.victim_rate,
                      a.victim_seconds, state)
        await relay.run()

    server = await asyncio.start_server(on_conn, "127.0.0.1", a.listen_port)
    # Issue #28 wire meter: the harness kills this proxy with SIGTERM the
    # moment the scenario ends — a canceled engine's FIN would otherwise sit
    # behind ~1 MB of unread paced data for the better part of an hour. Dump
    # the global relayed-bytes counter FIRST, then die.
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        try:
            loop.add_signal_handler(sig, lambda: _dump_and_die(state))
        except NotImplementedError:
            pass  # windows
    model = (f"UNIFORM every UploadPart PUT @ {a.victim_rate} B/s" if a.victim_part == "all"
             else f"victim partNumber={a.victim_part} @ {a.victim_rate} B/s, "
                  f"{a.victim_seconds}s, once")
    print(f"[throttle-proxy] listening 127.0.0.1:{a.listen_port} -> {target[0]}:{target[1]} ({model})",
          flush=True)
    async with server:
        await server.serve_forever()


def _dump_and_die(state: dict) -> None:
    print(f"[throttle-proxy] VICTIM-FINAL moved={state['wire_moved']}B", flush=True)
    os._exit(0)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("listen_port", type=int)
    ap.add_argument("target_host")
    ap.add_argument("target_port", type=int)
    ap.add_argument("--victim-part", type=str, default="2",
                    help="partNumber to impair once, or 'all' for the uniform "
                         "whole-uplink model (issue #28)")
    ap.add_argument("--victim-rate", type=int, default=96 * 1024)
    ap.add_argument("--victim-seconds", type=int, default=45)
    a = ap.parse_args()
    try:
        asyncio.run(main_async(a))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
