#!/usr/bin/env python3
"""Generate Dropbox content_hash test vectors for the #109 RED test.

Algorithm (official reference, dropbox.com/developers/reference/content-hash):
- empty input -> sha256("") hexdigest
- else: per 4 MiB block sha256 digest, concatenate digests, sha256 the
  concatenation, hexdigest.

Input pattern (reconstructible in Rust without embedding megabytes):
data[i] = (i % 251) as u8   (251 is prime, avoids 256-byte period alignment)
"""
import hashlib

BLOCK = 4 * 1024 * 1024


def content_hash(data: bytes) -> str:
    h = hashlib.sha256()
    if len(data) == 0:
        return h.hexdigest()
    n = (len(data) + BLOCK - 1) // BLOCK
    for i in range(n):
        blk = data[i * BLOCK:(i + 1) * BLOCK]
        h.update(hashlib.sha256(blk).digest())
    return h.hexdigest()


def gen(n: int) -> bytes:
    return bytes(i % 251 for i in range(n))


cases = {
    "empty": 0,
    "sub_block_1mib": 1024 * 1024,
    "exact_one_block_4mib": BLOCK,
    "block_plus_tail_5mib": BLOCK + 1024 * 1024,
    "three_blocks_tail_10mib_plus_1": 2 * BLOCK + 1024 * 1024 + 1,
}
for name, size in cases.items():
    print(f"{name} (len={size}): {content_hash(gen(size))}")
