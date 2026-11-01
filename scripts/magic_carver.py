#!/usr/bin/env python3
"""Carve embedded containers (zip/7z/MSI/CAB/PE/xz/zstd) out of a wrapper binary."""
import os
import sys

MAGICS = {
    b"PK\x03\x04": "zip",
    b"PK\x05\x06": "zip_eof",
    b"7z\xbc\xaf\x27\x1c": "7z",
    b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1": "cfb_msi",
    b"MSCF\x00\x00\x00\x00": "cab",
    b"\x1f\x8b\x08": "gzip",
    b"\xfd7zXZ\x00": "xz",
    b"\x28\xb5\x2f\xfd": "zstd",
    b"NullsoftInst": "nsis",
}


def sig_entropy_tail(data):
    return None


def main():
    path, outdir = sys.argv[1], sys.argv[2]
    os.makedirs(outdir, exist_ok=True)
    data = open(path, "rb").read()
    print(f"scanning {path} ({len(data):,} bytes)")
    found = {}
    for magic, label in MAGICS.items():
        start = 0
        offs = []
        while True:
            i = data.find(magic, start)
            if i < 0:
                break
            offs.append(i)
            start = i + 1
            if len(offs) > 40:
                break
        if offs:
            found[label] = offs
            print(f"{label:10s} x{len(offs):3d} first@ {offs[:8]}")

    # embedded PE (MZ followed by PE\0\0 within 0x200)
    pe_offs = []
    start = 0
    while True:
        i = data.find(b"MZ", start)
        if i < 0 or len(pe_offs) > 20:
            break
        if i + 0x40 < len(data) and data[i + 0x3C:i + 0x40] and i + 0x40 + 4 <= len(data):
            import struct
            try:
                e_lfanew = struct.unpack_from("<I", data, i + 0x3C)[0]
                if 0 < e_lfanew < 0x1000 and data[i + e_lfanew:i + e_lfanew + 4] == b"PE\x00\x00":
                    pe_offs.append(i)
            except Exception:
                pass
        start = i + 1
    if pe_offs:
        print(f"embedded PE x{len(pe_offs)} @ {pe_offs}")

    # carve: MSI (CFB), 7z, CAB, xz, zstd, gzip, nsis, biggest zips
    carve_plan = []
    for label in ("cfb_msi", "7z", "nsis", "cab", "xz", "zstd", "gzip"):
        if label in found:
            for off in found[label][:6]:
                carve_plan.append((off, label))
    if "zip" in found:
        big = sorted(found["zip"], reverse=True)[:3]  # last PK headers = central dir area
        carve_plan += [(off, "zip_tail") for off in big]
    for off, label in carve_plan:
        end = min(len(data), off + 40 * 1024 * 1024)
        blob = data[off:end]
        out = os.path.join(outdir, f"carve_{label}_{off:#x}.bin")
        open(out, "wb").write(blob)
        print(f"carved {label} @{off:#x} -> {out} ({len(blob):,} bytes)")


if __name__ == "__main__":
    main()
