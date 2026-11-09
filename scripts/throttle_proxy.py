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

REAL-NETWORK models (issue #32): a userspace relay CANNOT drop bytes inside
an established TCP stream (the stream must arrive complete and ordered or
HTTP itself breaks), so the models reproduce the APPLICATION-OBSERVABLE of
each impairment — the thing an uploader must actually survive:
- `--loss PCT`: each relayed segment independently, with probability PCT%,
  incurs an RTO-scale stall drawn U(0.2, 2.0) s, then continues. This IS
  what packet loss looks like to an uploader: multi-second progress freezes
  on a connection that stays alive, then the stream resumes. Each affected
  connection logs `LOSS stalls=<n> extra_s=<t>` when its body ends.
- `--latency-ms N --jitter-ms N`: every connection pays N±U(0,jitter) ms
  once at setup (connect RTT) and once before the response head is relayed
  back (response RTT) — ~2x RTT per request, bandwidth untouched.
- `--reset-after-bytes N`: ONE-SHOT. The first UploadPart PUT whose relayed
  request body reaches N bytes gets BOTH ends hard-reset (SO_LINGER=0 ->
  real RST, not FIN) and logs `RESET-WIRE moved=<N>B ...`. The latch
  guarantees the retry succeeds — the model probes ONE dead attempt.
Every sleep these models introduce is RESPONSIVE (<=250 ms steps, watching
the client for death) — the issue #28 lesson: a blind sleep hides the
engine's cancel behind minutes of pacing debt.

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
import random
import signal
import socket
import struct
import sys
import time

CHUNK = 64 * 1024
IDLE_TIMEOUT = 600


class SharedBucket:
    """One bearer's token bucket, SHARED by every connection (issue #55
    BearerCap shape): every body byte written upstream acquires tokens
    refilled continuously at `rate` B/s (2 s burst ceiling). The proxy runs
    on one asyncio loop — no locking; the 20 ms poll is 1.25 KB of
    granularity at the drill's 64 KiB/s. K concurrent sessions share the
    bucket, so the aggregate never exceeds the bearer — the exact opposite
    of the per-connection cap, where K flows each get their own C.
    """

    def __init__(self, rate_bps: float):
        self.rate = float(rate_bps)
        # One 20 ms poll-slice per grant — the fairness granule. The first
        # draft granted 250 ms of bucket per take (0.25), and the long-soak
        # dispatch (issue #95 R3, run 42541835145) caught the consequence:
        # the lead relay's take() drained the whole bucket every ~250 ms and
        # re-queued first, so ONE connection streamed at the full bearer
        # while the other three starved FLAT (their VICTIM-WIRE lines never
        # appeared; the engine's eviction saw honest 0.0 B/s and killed
        # them). At one poll-slice per grant the wake rotation shares the
        # bearer by construction: no single take can outrun a cycle.
        self.read_cap = max(1, int(self.rate * 0.02))
        self.tokens = 0.0  # starts EMPTY and never accumulates idle credit
        # beyond one poll slice — a MEASUREMENT instrument must show the
        # steady-state bearer shape, not a boot-time burst (caught live by
        # the #55 rehearsal: a 2 s ceiling handed the first PATCH 128 KiB
        # free and the "bearer" measured 2× its own rate).
        self.t0 = time.monotonic()

    def _refill(self) -> None:
        now = time.monotonic()
        self.tokens = min(self.tokens + (now - self.t0) * self.rate, self.read_cap)
        self.t0 = now

    async def take(self, n: int) -> None:
        while True:
            self._refill()
            if self.tokens >= n:
                self.tokens -= n
                return
            await asyncio.sleep(0.02)


class Relay:
    """One client connection: exactly one request/response pair."""

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                 target: tuple[str, int], victim_part: str, victim_rate: int,
                 victim_seconds: int, state: dict, latency_ms: int = 0,
                 jitter_ms: int = 0, loss_pct: float = 0.0,
                 reset_after_bytes: int = 0, per_conn_rate: int = 0,
                 bucket: "SharedBucket | None" = None):
        self.r = reader
        self.w = writer
        self.target = target
        self.victim_part = victim_part
        self.uniform = victim_part == "all"  # issue #28: the WHOLE uplink is slow
        self.victim_rate = victim_rate
        self.victim_seconds = victim_seconds
        self.state = state  # shared: {"victim_active_until", "victim_taken", "wire_moved", "reset_used"}
        # issue #55 measurement shapes: a per-connection cap (every body
        # request gets its own C — K flows sum to K·C) or one shared bearer
        # bucket (K flows share T — the aggregate is the bearer). They win
        # over the S3 victim selection: a measurement drill wants the WHOLE
        # link shaped, not one part.
        self.per_conn_rate = per_conn_rate
        self.bucket = bucket
        self.flow_active = False  # this request's body is being paced
        # issue #32 real-network models
        self.latency_s = latency_ms / 1000.0
        self.jitter_s = jitter_ms / 1000.0
        self.loss_pct = loss_pct
        self.reset_after = reset_after_bytes
        self.stalls = 0
        self.stall_extra_s = 0.0

    async def _responsive_sleep(self, seconds: float,
                                src: asyncio.StreamReader | None) -> bool:
        """Sleep `seconds` in <=250 ms steps, watching `src` for client death
        (the #28 lesson: a blind sleep hides the engine's cancel behind the
        debt). Returns False if the peer died mid-sleep — the caller bails.
        """
        deadline = time.monotonic() + seconds
        while True:
            now = time.monotonic()
            if now >= deadline:
                return True
            if src is not None and (src.exception() is not None or src.at_eof()):
                return False
            await asyncio.sleep(min(0.25, deadline - now))

    async def _maybe_stall(self, src: asyncio.StreamReader) -> bool:
        """Issue #32 loss model: with probability loss_pct%, incur one
        RTO-scale stall (U(0.2, 2.0) s) before this segment moves. Returns
        False if the client died during the stall.
        """
        if self.loss_pct <= 0.0 or random.random() * 100.0 >= self.loss_pct:
            return True
        stall = random.uniform(0.2, 2.0)
        self.stalls += 1
        self.stall_extra_s += stall
        return await self._responsive_sleep(stall, src)

    def _rtt_pause(self) -> float:
        """One RTT-sized pause (issue #32 latency model): N±U(0, jitter)."""
        d = self.latency_s + (random.uniform(0, self.jitter_s) if self.jitter_s else 0.0)
        if d > 0:
            time.sleep(d)
        return d

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
                           until: int | None, rate: int | None, pre: bytes,
                           reset_at: int | None = None) -> int:
        # NOTE: the shared-bucket mode (issue #55) rides this same relay with
        # `rate=None` and `self.bucket` set — the pacing happens at the WRITE
        # (take tokens before each chunk), not at the read schedule.
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
        # `pre` is dripped through the schedule like everything else (kept in
        # the stash): relaying the whole head-glued burst at t=0 puts ~64 KB
        # upstream in one blast and then ~32 s of SILENCE at B/s rates — the
        # volume server's idle timeout (default 30 s, cloud RED 41571828134)
        # kills the request mid-body on connection 1 exactly as it kills
        # bursty reads later. Smooth ≤1 s flow from byte zero.
        stash = bytearray(pre)
        if pre and rate is None and self.bucket is None:
            dst.write(pre)
            await dst.drain()
            moved += len(pre)
            stash.clear()
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
            # due: relay ~1 s of wire time (`rate` bytes) — a 64 KB burst at
            # B/s rates means ~32 s of SILENCE on the upstream connection and
            # the volume server's idle timeout (default 30 s — cloud RED
            # 41571828134) kills the request mid-body. Smooth ≤1 s flow keeps
            # the server alive AND the engine's progress meter advancing, so
            # the resplit windows measure the real link. Bucket mode reads
            # only ≤ 250 ms of the shared bearer so one connection cannot
            # hoard the whole bucket while its peers starve (issue #55).
            if rate:
                cap = max(rate, 1)
            elif self.bucket is not None:
                cap = self.bucket.read_cap
            else:
                cap = CHUNK
            n = min(CHUNK, cap)
            if until is not None:
                n = min(n, until - moved)
            if stash:
                b, stash = bytes(stash[:n]), stash[n:]
            else:
                try:
                    b = await asyncio.wait_for(src.read(n), timeout=0.25)
                except asyncio.TimeoutError:
                    continue  # schedule is due but the wire is idle — re-check
                except (ConnectionResetError, OSError):
                    return moved  # RST surfaced through the read itself
                if b == b"":
                    return moved  # the client is gone (EOF at the transport)
            if not await self._maybe_stall(src):
                return moved  # the client died inside an RTO stall (#32 loss)
            if self.bucket is not None:
                # The shared bearer paces the WRITES: tokens before bytes,
                # so the aggregate across every connection never exceeds it.
                await self.bucket.take(len(b))
            dst.write(b)
            await dst.drain()
            moved += len(b)
            if rate or self.bucket is not None:
                self.state["wire_moved"] += len(b)
            # Issue #32 reset model: one-shot hard RST of BOTH ends the moment
            # this UploadPart PUT's relayed body crosses `reset_at`. Bytes past
            # the threshold die in flight — the engine sees a real mid-body
            # connection reset (ConnectionResetError), never a clean FIN.
            if reset_at is not None and moved >= reset_at and not self.state["reset_used"]:
                self.state["reset_used"] = True
                print(f"[throttle-proxy] RESET-WIRE moved={moved}B {self.last_req} "
                      f"— hard RST both ends (one-shot, issue #32)", flush=True)
                for wr in (dst, self.w):
                    try:
                        sock = wr.get_extra_info("socket")
                        if sock is not None:
                            sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                                            struct.pack("ii", 1, 0))
                    except OSError:
                        pass
                    try:
                        wr.close()
                    except OSError:
                        pass
                return moved
        # The schedule paces READS, and a body that fits in the socket buffer
        # would otherwise cross faster than its cap (caught live by the #55
        # drill: a 64 KiB canary measured 4x its own per-connection cap —
        # one 64 KiB read, zero schedule waits). The relay may only return
        # once the wire's debt is paid: the end-to-end rate IS the cap for
        # bodies of ANY size. Bucket mode already paces the writes (tokens
        # before bytes) and needs no debt accounting.
        if rate and moved:
            behind = start + moved / rate - time.monotonic()
            while behind > 0:
                if src.exception() is not None or src.at_eof():
                    break  # the client is gone — nothing left to pace for it
                await asyncio.sleep(min(0.25, behind))
                behind = start + moved / rate - time.monotonic()
        return moved

    async def pump(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                   count: int, rate: int | None, pre: bytes = b"",
                   reset_at: int | None = None) -> int:
        """Move exactly `count` body bytes; pace reads when a rate is set.

        `pre` carries body bytes ALREADY consumed from the wire (they arrived
        glued to the request head in one TCP read). They are written first and
        count toward the pacing budget — dropping them deadlocks both sides:
        the proxy then waits for bytes the client already sent, while the
        client waits for a response that can never complete.
        """
        return await self._paced_relay(src, dst, count, rate, pre, reset_at)

    async def pump_until_eof(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter,
                             rate: int | None, pre: bytes = b"",
                             reset_at: int | None = None) -> int:
        return await self._paced_relay(src, dst, None, rate, pre, reset_at)

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
        self.last_req = f"{method} {path}"
        length = int(headers.get("content-length", "0"))
        te = "chunked" in headers.get("transfer-encoding", "").lower()
        print(f"[throttle-proxy] req {request_line}", flush=True)

        # --- issue #32 latency model: the connect RTT -----------------------
        if self.latency_s > 0:
            d = self._rtt_pause()
            print(f"[throttle-proxy] LATENCY connect {d * 1000:.0f}ms {method} {path}",
                  flush=True)

        # --- victim selection -------------------------------------------------
        # issue #55 measurement shapes FIRST: a per-connection cap paces every
        # body-carrying request (tus PATCH eats the cap; creations/OPTIONS/
        # HEAD ride free — exactly the per-flow-cap shape the prober's 1-vs-K
        # question is about); the shared bucket paces nothing here — its take
        # happens at the write inside _paced_relay.
        rate: int | None = None
        has_body = te or length > 0
        if self.per_conn_rate and has_body:
            rate = self.per_conn_rate
            self.flow_active = True
            print(f"[throttle-proxy] FLOW: {request_line} -> {self.per_conn_rate} B/s "
                  f"(per-connection cap, issue #55)", flush=True)
        elif self.bucket is not None and has_body:
            self.flow_active = True
            print(f"[throttle-proxy] FLOW: {request_line} -> shared bearer "
                  f"(bucket mode, issue #55)", flush=True)

        # uniform mode: every UploadPart PUT is a slow-link body, no latch, no
        # expiry — the scenario ends when the engine cancels (issue #28).
        # victim mode: the FIRST connection EVER carrying the victim part.
        is_part_put = "partNumber=" in path and method == "PUT"
        if rate is None and self.bucket is None and self.uniform and is_part_put:
            rate = self.victim_rate
            print(f"[throttle-proxy] UNIFORM: {request_line} -> {self.victim_rate} B/s "
                  f"(binds to this connection's body)", flush=True)
        elif (rate is None and self.bucket is None and not self.uniform
                and (f"partNumber={self.victim_part}" in path and method == "PUT"
                     and not self.state["victim_taken"])):
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
        # reset_at arms the #32 one-shot RST for UploadPart PUTs only.
        t_body0 = time.monotonic()
        req_moved = 0
        reset_at = (self.reset_after
                    if self.reset_after > 0 and is_part_put
                    and not self.state["reset_used"] else None)
        if te:
            try:
                req_moved = await self.pump_until_eof(self.r, uwriter, rate, pre=rest,
                                                      reset_at=reset_at)  # best effort
            except OSError as e:
                # The upstream answered EARLY and closed (tusd 409s a stale
                # offset without reading the body) — honest log, then tear
                # the client side down too (a silent close would look like a
                # proxy bug; caught live by the #55 rehearsal fact 3b).
                print(f"[throttle-proxy] UPSTREAM-GONE mid-body ({e}) "
                      f"{request_line} — after moved={req_moved}B", flush=True)
                for wr in (uwriter, self.w):
                    try:
                        wr.close()
                    except OSError:
                        pass
                return
        elif length:
            try:
                req_moved = await self.pump(self.r, uwriter, length, rate, pre=rest,
                                            reset_at=reset_at)
            except OSError as e:
                print(f"[throttle-proxy] UPSTREAM-GONE mid-body ({e}) "
                      f"{request_line} — after moved={req_moved}B", flush=True)
                for wr in (uwriter, self.w):
                    try:
                        wr.close()
                    except OSError:
                        pass
                return
            if req_moved != length:
                # The client abandoned this connection mid-body — this is the
                # engine's eviction/resplit/cancel tearing the impaired attempt
                # off the wire. The server can never finish this request; relay
                # nothing. Close BOTH ends now so the upstream sees a real
                # disconnect instead of a proxy that quietly waits forever.
                print(f"[throttle-proxy] client abandoned {request_line.split(' ')[0]} "
                      f"{path} mid-body (moved {req_moved} of {length} B) — tearing the link down", flush=True)
                if rate or self.flow_active:
                    el = time.monotonic() - t_body0
                    tag = "VICTIM-WIRE" if rate and not self.flow_active else "FLOW-WIRE"
                    print(f"[throttle-proxy] {tag} moved={req_moved}B in {el:.1f}s "
                          f"({req_moved / el:.0f} B/s) {method} {path}", flush=True)
                for wr in (uwriter, self.w):
                    try:
                        wr.close()
                    except OSError:
                        pass
                return
        if rate or self.flow_active:
            el = time.monotonic() - t_body0
            tag = "VICTIM-WIRE" if rate and not self.flow_active else "FLOW-WIRE"
            print(f"[throttle-proxy] {tag} moved={req_moved}B in {el:.1f}s "
                  f"({req_moved / el:.0f} B/s) {method} {path}", flush=True)
        if self.stalls:
            print(f"[throttle-proxy] LOSS stalls={self.stalls} "
                  f"extra_s={self.stall_extra_s:.2f} {method} {path}", flush=True)

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
            # --- issue #32 latency model: the response RTT ------------------
            if self.latency_s > 0:
                d = self._rtt_pause()
                print(f"[throttle-proxy] LATENCY response {d * 1000:.0f}ms {method} {path}",
                      flush=True)
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
    state = {"victim_taken": False, "victim_active_until": None, "wire_moved": 0,
             "reset_used": False}
    target = (a.target_host, a.target_port)
    # issue #55: one bucket object is shared by EVERY connection — that
    # sharing IS the bearer model (K flows, one T).
    bucket = SharedBucket(a.total_rate) if a.total_rate else None

    async def on_conn(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        relay = Relay(reader, writer, target, a.victim_part, a.victim_rate,
                      a.victim_seconds, state, latency_ms=a.latency_ms,
                      jitter_ms=a.jitter_ms, loss_pct=a.loss,
                      reset_after_bytes=a.reset_after_bytes,
                      per_conn_rate=a.per_conn_rate, bucket=bucket)
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
    if a.per_conn_rate:
        model = f"PER-CONN every body request @ {a.per_conn_rate} B/s (issue #55)"
    elif a.total_rate:
        model = f"SHARED-BEARER all connections @ {a.total_rate} B/s total (issue #55)"
    elif a.victim_part == "all":
        model = f"UNIFORM every UploadPart PUT @ {a.victim_rate} B/s"
    else:
        model = (f"victim partNumber={a.victim_part} @ {a.victim_rate} B/s, "
                 f"{a.victim_seconds}s, once")
    net = []
    if a.loss:
        net.append(f"loss {a.loss}% of segments -> RTO-scale stall U(0.2,2.0)s")
    if a.latency_ms:
        net.append(f"latency {a.latency_ms}ms +-{a.jitter_ms}ms (connect+response)")
    if a.reset_after_bytes:
        net.append(f"one-shot RST after {a.reset_after_bytes} B of a part PUT")
    if net:
        model += " | " + "; ".join(net)
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
                    help="partNumber to impair once, 'all' for the uniform "
                         "whole-uplink model (issue #28), or 'none' to disable "
                         "victim selection entirely (the latency/reset models "
                         "must pass 'none' — any other value leaves the "
                         "victim-rate default ARMED and contaminates the leg, "
                         "caught live by s7net[rtt] first run, issue #32)")
    ap.add_argument("--victim-rate", type=int, default=96 * 1024)
    ap.add_argument("--victim-seconds", type=int, default=45)
    # --- issue #55 measurement shapes ---------------------------------------
    ap.add_argument("--per-conn-rate", type=int, default=0, metavar="BPS",
                    help="pace EVERY body-carrying request to BPS per connection "
                         "(any method/path — the tus per-flow-cap shape: K "
                         "concurrent flows each get C and sum to K·C; creations, "
                         "OPTIONS and HEAD carry no body and ride free) "
                         "(issue #55)")
    ap.add_argument("--total-rate", type=int, default=0, metavar="BPS",
                    help="ALL connections share ONE token bucket of BPS — the "
                         "shared-bearer shape: the aggregate never exceeds the "
                         "bearer no matter how many flows (issue #55)")
    # --- issue #32 real-network models -------------------------------------
    ap.add_argument("--loss", type=float, default=0.0, metavar="PCT",
                    help="probability PER RELAYED SEGMENT of an RTO-scale stall "
                         "U(0.2,2.0)s — the application-observable of packet loss "
                         "(a userspace relay cannot drop in-stream bytes; this "
                         "reproduces what loss does to an uploader: freezes, then "
                         "the stream resumes) (issue #32)")
    ap.add_argument("--latency-ms", type=int, default=0, metavar="N",
                    help="RTT modeled as one N ms pause per connection at setup "
                         "and one before the response head is relayed back "
                         "(issue #32)")
    ap.add_argument("--jitter-ms", type=int, default=0, metavar="N",
                    help="uniform 0..N ms jitter added to each latency pause "
                         "(issue #32)")
    ap.add_argument("--reset-after-bytes", type=int, default=0, metavar="N",
                    help="ONE-SHOT: hard-RST both ends when the first UploadPart "
                         "PUT's relayed body crosses N bytes — a real mid-body "
                         "connection reset (issue #32)")
    a = ap.parse_args()
    try:
        asyncio.run(main_async(a))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
