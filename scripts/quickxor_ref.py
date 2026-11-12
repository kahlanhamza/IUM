#!/usr/bin/env python3
"""quickXorHash — INDEPENDENT implementation from the official high-level description
(learn.microsoft.com 'Code Snippets: QuickXorHash Algorithm').

Semantics implemented from the ALGORITHM EXPLANATION (not transliterated from C#):
- 160-bit circular vector; byte at absolute index n is XORed at bit position
  (n*11) mod 160.
- Final: XOR the 8-byte little-endian length into bytes 12..20 of the 20-byte
  little-endian serialization (= bits 96..160).
- Output: 20 bytes -> standard base64 (28 chars) — the Graph `quickXorHash` string.

Cross-checked against rclone's independent Go implementation (rclone hashsum
quickxorhash) before the vectors are pinned into the RED contract test.
"""
import base64
import sys

WIDTH = 160


def _xor_bit(vec: int, b: int, p: int) -> int:
    """XOR byte b into the 160-bit circular vector at bit position p.

    The C# reference splits the byte across cells and the high part XORs
    into the next cell (index2 = isLastCell ? 0 : ...): bits >= WIDTH come
    back around to bit 0 — and they XOR, they do not OR (the C# uses `^=`
    on both cells; an OR here makes the fold order-dependent and wrong).
    """
    full = vec ^ (b << p)
    return (full & ((1 << WIDTH) - 1)) ^ (full >> WIDTH)


def _xorhash0(data: bytes) -> int:
    vec = 0
    for i, b in enumerate(data):
        vec = _xor_bit(vec, b, (i * 11) % WIDTH)
    return vec


def quickxor(data: bytes) -> bytes:
    """20 raw bytes."""
    vec = _xorhash0(data)
    vec ^= (len(data) & ((1 << 64) - 1)) << 96  # length into bytes 12..20
    return vec.to_bytes(20, "little")


def quickxor_b64(data: bytes) -> str:
    return base64.b64encode(quickxor(data)).decode()


def quickxor_streaming(chunks) -> bytes:
    """Streaming variant over arbitrary chunk sizes (the adapter's real call pattern)."""
    vec = 0
    shift = 0
    length = 0
    for chunk in chunks:
        cb = len(chunk)
        # the C# reference updates _shiftSoFar ONCE per HashCore call, after
        # the i-loop — not per byte.
        for i in range(min(cb, WIDTH)):
            p = (shift + i * 11) % WIDTH
            for j in range(i, cb, WIDTH):
                vec = _xor_bit(vec, chunk[j], p)
        shift = (shift + 11 * (cb % WIDTH)) % WIDTH
        length += cb
    vec ^= (length & ((1 << 64) - 1)) << 96
    return vec.to_bytes(20, "little")


def pattern(n: int) -> bytes:
    return bytes(i % 251 for i in range(n))


if __name__ == "__main__":
    cases = [
        ("empty", b""),
        ("one-byte-a", b"a"),
        ("three-byte-abc", b"abc"),
        ("63B", pattern(63)),
        ("64B", pattern(64)),
        ("65B", pattern(65)),
        ("159B", pattern(159)),
        ("160B", pattern(160)),
        ("161B", pattern(161)),
        ("600B", pattern(600)),
        ("1MiB", pattern(1 * 1024 * 1024)),
        ("5MiB", pattern(5 * 1024 * 1024)),
    ]
    out = []
    for name, data in cases:
        h = quickxor_b64(data)
        hs = base64.b64encode(quickxor_streaming([data[k:k + 700_000] for k in range(0, len(data), 700_000)])).decode()
        assert h == hs, f"streaming mismatch on {name}: {h} vs {hs}"
        out.append((name, h))
        print(f"{name}: {h}")
    with open("/home/z/my-project/scripts/quickxor_vectors.txt", "w") as f:
        for name, h in out:
            f.write(f"{name}\t{h}\n")
    print("vectors written to scripts/quickxor_vectors.txt")
