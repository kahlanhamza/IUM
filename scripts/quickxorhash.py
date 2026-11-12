#!/usr/bin/env python3
"""quickXorHash pure module — IUM #110 RED vector source.

Three INDEPENDENT formulations of Microsoft's published quickXorHash
(MS Learn code-snippets/quickxorhash C# reference + rclone Go package):

1. slot  — per-position-class XOR (p mod 160), rotate each class value by
   (s*11) mod 160 inside the 160-bit circular space, XOR length, LE bytes.
   Algebraic basis: rotation is linear over XOR, and p*11 ≡ s*11 (mod 160)
   whenever p ≡ s (mod 160).
2. ring  — rclone's structure: XOR the raw stream into a 1760-byte ring
   (1760*11 ≡ 0 mod 160 keeps every ring cell's shift coherent), then
   per-cell shift-OR into a 21-byte checksum, wrap h[20] -> h[0], XOR the
   8-byte LE length into h[12..19].
3. naive — literal pseudocode: per byte i, XOR rotate(extend8(b), i*11) on
   a 160-bit integer accumulator, byte-reverse (BE reversed == LE), XOR
   the length as the low 64 bits.

All three must agree byte-for-byte on: the official vectors (rclone's
quickxorhash_test.go suite, itself derived from Microsoft's original test
data) + randomized sizes around the structural boundaries (159/160/161,
1759/1760/1761 ring wrap, 3200, 3520).

Output encoding: standard base64 (WITH padding) of the 20 bytes — the wire
form Graph returns in `file.hashes.quickXorHash`.
"""
import base64
import random

MASK160 = (1 << 160) - 1
WIDTH = 160
SHIFT = 11


def _xor_len_le(out: bytearray, n: int) -> bytearray:
    """The 8-byte LE length is XORed into out[12..19] — the C# reference
    (rgb[WidthInBits/8 - 8 + i] ^= lengthBytes[i]) and rclone's Go
    (h[Size-8+k] ^= d>>(8k)) agree; the pseudocode's reverse() is what
    flips the block's low bits to the array's high bytes."""
    d = n & 0xFFFFFFFFFFFFFFFF
    for k in range(8):
        out[12 + k] ^= (d >> (8 * k)) & 0xFF
    return out


def quick_xor_hash_slot(data: bytes) -> bytes:
    slots = [0] * WIDTH
    for p, b in enumerate(data):
        slots[p % WIDTH] ^= b
    acc = 0
    for s in range(WIDTH):
        v = slots[s]
        if v:
            sh = (s * SHIFT) % WIDTH
            acc ^= ((v << sh) | (v >> (WIDTH - sh))) & MASK160
    return bytes(_xor_len_le(bytearray(acc.to_bytes(20, "little")), len(data)))


def quick_xor_hash_ring(data: bytes) -> bytes:
    DS = SHIFT * WIDTH  # 1760
    ring = bytearray(DS)
    for p, b in enumerate(data):
        ring[p % DS] ^= b
    h = bytearray(21)
    for i in range(DS):
        shift = (i * SHIFT) % WIDTH
        shifted = ring[i] << (shift % 8)
        h[shift // 8] ^= shifted & 0xFF
        h[shift // 8 + 1] ^= (shifted >> 8) & 0xFF
    h[0] ^= h[20]
    d = len(data)
    for k in range(8):
        h[WIDTH // 8 - 8 + k] ^= (d >> (8 * k)) & 0xFF
    return bytes(h[:20])


def quick_xor_hash_naive(data: bytes) -> bytes:
    """Pseudocode shape: per byte i, XOR rotate(extend8(b), i*11) into the
    160-bit accumulator — INDEPENDENT of slot's per-position-class trick,
    which is what this variant cross-checks. Serialization + length placement
    are pinned by the executable references (C# rgb / Go h) and the official
    vectors: LE byte array, length XORed into bytes 12..19. The pseudocode's
    reverse() is the notational bridge between its big-endian block
    convention and that output array — no separate step survives it."""
    acc = 0
    for i, b in enumerate(data):
        sh = (i * SHIFT) % WIDTH
        acc ^= ((b << sh) | (b >> (WIDTH - sh))) & MASK160
    return bytes(_xor_len_le(bytearray(acc.to_bytes(20, "little")), len(data)))


def quick_xor_hash(data: bytes) -> bytes:
    return quick_xor_hash_slot(data)


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


# Official vectors (rclone backend/onedrive/quickxorhash/quickxorhash_test.go,
# single-line entries only — input b64, output b64).
OFFICIAL = [
    (0, "", "AAAAAAAAAAAAAAAAAAAAAAAAAAA="),
    (1, "Sg==", "SgAAAAAAAAAAAAAAAQAAAAAAAAA="),
    (2, "tbQ=", "taAFAAAAAAAAAAAAAgAAAAAAAAA="),
    (3, "0pZP", "0rDEEwAAAAAAAAAAAwAAAAAAAAA="),
    (4, "jRRDVA==", "jaDAEKgAAAAAAAAABAAAAAAAAAA="),
    (5, "eAV52qE=", "eChAHrQRCgAAAAAABQAAAAAAAAA="),
    (6, "luBZlaT6", "lgBHFipBCn0AAAAABgAAAAAAAAA="),
    (7, "qaApEj66lw==", "qQBFCiTgA11cAgAABwAAAAAAAAA="),
    (8, "/aNzzCFPS/A=", "/RjFHJgRgicsAR4ACAAAAAAAAAA="),
    (9, "n6Neh7p6fFgm", "nxiFFw6hCz3wAQsmCQAAAAAAAAA="),
    (10, "J9iPGCbfZSTNyw==", "J8DGIzBggm+UgQTNUgYAAAAAAAA="),
    (16, "2fuxhBJXtpWFe8dOfdGeHw==", "O9tHLAghgSvYohKFyMMxnNCHaHg="),
    (32, "NbYXsp5/K6mR+NmHwExjvWeWDJFnXTKWVlzYHoesp2E=", "wjuAuWDiq04qDt1R8hHWDDcwVoQ="),
    (48, "VXs4t4tfXGiWAL6dlhEMm0YQF0f2w9rzX0CvIVeuW56o6/ec2auMpKeU2VeteEK5", "sq24lSf7wXLH8eigHl07X+qPTps="),
]

# Structural boundary sizes for the three-way cross-check.
BOUNDARIES = [159, 160, 161, 1759, 1760, 1761, 3200, 3520, 5000]


def run_selftest() -> None:
    impls = {
        "slot": quick_xor_hash_slot,
        "ring": quick_xor_hash_ring,
        "naive": quick_xor_hash_naive,
    }
    rng = random.Random(1100)
    random_cases = []
    for n in BOUNDARIES:
        random_cases.append((n, bytes(rng.randrange(256) for _ in range(n))))
    for n in [rng.randrange(1, 2000) for _ in range(12)]:
        random_cases.append((n, bytes(rng.randrange(256) for _ in range(n))))

    fails = 0
    for name, fn in impls.items():
        for size, inp, want in OFFICIAL:
            got = b64(fn(base64.b64decode(inp)))
            if got != want:
                print(f"FAIL {name} official size={size}: {got} != {want}")
                fails += 1
        for n, data in random_cases:
            ref = b64(quick_xor_hash_slot(data))
            got = b64(fn(data))
            if got != ref:
                print(f"FAIL {name} random size={n}: {got} != {ref}")
                fails += 1
    total = len(OFFICIAL) * 3 + len(random_cases) * 3
    print(f"selftest: {total - fails}/{total} passed" + ("" if fails == 0 else f", {fails} FAILED"))
    if fails:
        raise SystemExit(1)


if __name__ == "__main__":
    run_selftest()
