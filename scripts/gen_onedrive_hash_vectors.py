#!/usr/bin/env python3
"""Generate OneDrive quickXorHash test vectors for the #110 RED contract test.

Pattern is reconstructible in Rust without embedding megabytes (the #109
pattern): data[i] = (i % 251) as u8 — 251 is prime, avoiding 256-byte
period alignment with the 160-byte rotation ring and the 1760-byte
rclone ring.

Sizes chosen around the lane's REAL structure:
- 0            empty (length-only hash)
- 160 / 161    the rotation ring period and period+1
- 327_680      exactly one 320 KiB Graph fragment (upload geometry)
- 1 MiB        sub-uniform-block
- 5 MiB        the prescribed uniform block (16 x 320 KiB)
- 2*4 MiB + 1 MiB + 1   multi-block odd tail (mirrors the #109 case shape)
- 12 MiB + 123 multi-ring-wrap, odd tail

Every vector is computed with the slot formulation and cross-checked
against the ring formulation (sizes <= 1_200_000; same logic, different
structure — the three-way selftest in quickxorhash.py pins the algebra).
Also re-runs the official-vector selftest first.
"""
import base64
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from quickxorhash import OFFICIAL, quick_xor_hash_ring, quick_xor_hash_slot, run_selftest  # noqa: E402

CASES = [
    ("empty", 0),
    ("rotation_period", 160),
    ("rotation_period_plus_1", 161),
    ("one_320kib_fragment", 327_680),
    ("sub_block_1mib", 1024 * 1024),
    ("uniform_block_5mib", 5 * 1024 * 1024),
    ("multi_block_odd_tail", 2 * 4 * 1024 * 1024 + 1024 * 1024 + 1),
    ("multi_ring_wrap_odd_tail", 12 * 1024 * 1024 + 123),
]


def pattern(n: int) -> bytes:
    return bytes(i % 251 for i in range(n))


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def hexs(data: bytes) -> str:
    return data.hex()


def main() -> None:
    run_selftest()
    print("--- pattern vectors (Rust: pattern(len) = (0..len).map(|i| (i % 251) as u8)) ---")
    for name, n in CASES:
        data = pattern(n)
        want = quick_xor_hash_slot(data)
        if n <= 1_200_000:
            ref = quick_xor_hash_ring(data)
            assert want == ref, f"slot/ring disagree at {name}"
        print(f'// {name} (len={n})')
        print(f'assert_eq!(quick_xor_hash(&pattern({n})), "{b64(want)}");')
    print("--- official tiny vectors (hex input, base64 output) ---")
    for size, inp, want in OFFICIAL:
        data = base64.b64decode(inp)
        got = quick_xor_hash_slot(data)
        assert b64(got) == want, f"official regression at size {size}"
        print(f'// official vector size={size}')
        print(f'assert_eq!(quick_xor_hash(&[{", ".join(f"0x{b:02x}" for b in data)}]), "{want}");')


if __name__ == "__main__":
    main()
