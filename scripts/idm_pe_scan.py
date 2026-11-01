#!/usr/bin/env python3
"""
Clean-room static scanner for Windows PE binaries (IDM study).
Reads ONLY: PE headers, section metadata, import tables, and string constants.
Does NOT decompile, disassemble, unpack, or reproduce any code.
Outputs a JSON dump + a markdown dossier.
"""
import json
import math
import os
import re
import struct
import sys
from collections import defaultdict

import pefile


def entropy(data: bytes) -> float:
    if not data:
        return 0.0
    counts = [0] * 256
    for b in data:
        counts[b] += 1
    ent = 0.0
    n = len(data)
    for c in counts:
        if c:
            p = c / n
            ent -= p * math.log2(p)
    return round(ent, 3)


KEYWORDS = {
    "http_headers": [
        "Range:", "Content-Range", "Accept-Ranges", "Content-Length",
        "Content-Type", "Last-Modified", "If-Modified-Since", "If-Range",
        "ETag", "Expect:", "Connection", "Keep-Alive", "User-Agent",
        "Authorization", "Cookie", "Proxy-", "multipart", "chunked",
    ],
    "protocols": [
        "http://", "https://", "ftp://", "mms://", "rtsp://", "ftps://",
        "sftp", "torrent", "magnet:",
    ],
    "transfer_core": [
        "segment", "chunk", "part", "resume", "restart", "retry",
        "speed", "bandwidth", "connection", "thread", "download",
        "upload", "schedule", "queue",
    ],
    "socket_io": [
        "WSASocket", "WSAEvent", "WSARecv", "WSASend", "getaddrinfo",
        "gethostbyname", "setsockopt", "ioctlsocket", "overlapped",
    ],
    "wininet_api": [
        "InternetOpen", "InternetConnect", "HttpOpenRequest", "HttpSendRequest",
        "InternetReadFile", "InternetSetFilePointer", "HttpAddRequestHeaders",
        "InternetQueryDataAvailable", "FtpOpenFile", "FtpGetFile",
    ],
    "file_io": [
        "SetFilePointer", "WriteFile", "ReadFile", "CreateFile", "FlushFile",
        "SetEndOfFile", "CreateFileMapping", "MapViewOfFile", "Overlapped",
    ],
}


def extract_ascii(data: bytes, min_len=6):
    pat = re.compile(rb"[\x20-\x7e]{%d,}" % min_len)
    return [m.group().decode("ascii", "ignore") for m in pat.finditer(data)]


def extract_utf16(data: bytes, min_len=6):
    out = []
    pat = re.compile(rb"(?:[\x20-\x7e]\x00){%d,}" % min_len)
    for m in pat.finditer(data):
        try:
            out.append(m.group().decode("utf-16-le"))
        except Exception:
            pass
    return out


def main():
    path = sys.argv[1]
    outdir = sys.argv[2]
    os.makedirs(outdir, exist_ok=True)
    pe = pefile.PE(path, fast_load=False)
    name = os.path.basename(path)

    info = {
        "file": name,
        "size_bytes": os.path.getsize(path),
        "machine": hex(pe.FILE_HEADER.Machine),
        "time_stamp": pe.FILE_HEADER.TimeDateStamp,
        "linker_version": f"{pe.OPTIONAL_HEADER.MajorLinkerVersion}.{pe.OPTIONAL_HEADER.MinorLinkerVersion}",
        "subsystem": pe.OPTIONAL_HEADER.Subsystem,
        "entry_point": hex(pe.OPTIONAL_HEADER.AddressOfEntryPoint),
        "is_dotnet": bool(pe.OPTIONAL_HEADER.DATA_DIRECTORY[14].VirtualAddress),
        "sections": [],
        "imports": {},
        "exports_count": 0,
    }

    for s in pe.sections:
        nm = s.Name.rstrip(b"\x00").decode("ascii", "ignore")
        info["sections"].append({
            "name": nm,
            "virtual_size": s.Misc_VirtualSize,
            "virtual_address": hex(s.VirtualAddress),
            "raw_size": s.SizeOfRawData,
            "entropy": round(s.get_entropy(), 3),
        })

    if hasattr(pe, "DIRECTORY_ENTRY_IMPORT"):
        for entry in pe.DIRECTORY_ENTRY_IMPORT:
            dll = entry.dll.decode("ascii", "ignore")
            funcs = []
            for imp in entry.imports:
                if imp.name:
                    funcs.append(imp.name.decode("ascii", "ignore"))
                else:
                    funcs.append(f"ord_{imp.ordinal}")
            info["imports"][dll] = funcs
    if hasattr(pe, "DIRECTORY_ENTRY_EXPORT"):
        info["exports_count"] = len(pe.DIRECTORY_ENTRY_EXPORT.symbols)

    data = open(path, "rb").read()
    ascii_strs = extract_ascii(data)
    utf16_strs = extract_utf16(data)
    all_strs = ascii_strs + utf16_strs

    hits = {}
    for category, kws in KEYWORDS.items():
        cat_hits = {}
        for kw in kws:
            kw_l = kw.lower()
            matched = [s for s in all_strs if kw_l in s.lower()]
            if matched:
                cat_hits[kw] = {"count": len(matched), "samples": matched[:6]}
        if cat_hits:
            hits[category] = cat_hits

    interesting_mutex_like = sorted(
        {s for s in all_strs if re.search(r"(?i)\b(idm|mutex|global\\\\|local\\\\)", s) and 4 < len(s) < 60}
    )[:40]

    result = {"pe_info": info, "keyword_hits": hits, "mutex_like_strings": interesting_mutex_like,
              "strings_total_ascii": len(ascii_strs), "strings_total_utf16": len(utf16_strs)}

    with open(os.path.join(outdir, name + ".scan.json"), "w") as f:
        json.dump(result, f, indent=1)

    # markdown dossier
    md = []
    md.append(f"# Clean-room static scan: `{name}`\n")
    md.append(f"- Size: {info['size_bytes']:,} bytes | Machine: {info['machine']} (0x14c=i386, 0x8664=x64) "
              f"| Subsystem: {info['subsystem']} (2=GUI, 3=CLI) | .NET: {info['is_dotnet']}")
    md.append(f"- Linker: {info['linker_version']} | Entry: {info['entry_point']} | Exports: {info['exports_count']}\n")
    md.append("## Sections\n")
    md.append("| Section | Virtual size | Raw size | Entropy | Note |")
    md.append("|---|---|---|---|---|")
    for s in info["sections"]:
        note = ""
        if s["entropy"] >= 7.2:
            note = "HIGH entropy (packed/compressed?)"
        elif s["name"] == ".ndata":
            note = "NSIS installer data"
        md.append(f"| {s['name']} | {s['virtual_size']:,} | {s['raw_size']:,} | {s['entropy']} | {note} |")
    md.append("\n## Import table (DLLs and function counts)\n")
    md.append("| DLL | Functions | Transfer-relevant |")
    md.append("|---|---|---|")
    relevant = ("ws2_32", "wininet", "winhttp", "crypt32", "bcrypt", "ncrypt", "wintrust")
    for dll, funcs in sorted(info["imports"].items()):
        rel = ""
        if any(r in dll.lower() for r in relevant):
            rel = ", ".join(funcs[:14]) + (" ..." if len(funcs) > 14 else "")
        md.append(f"| {dll} | {len(funcs)} | {rel} |")
    md.append("\n## Keyword evidence (strings)\n")
    for cat, kws in hits.items():
        md.append(f"\n### {cat}\n")
        for kw, v in kws.items():
            samples = "; ".join(x.replace("|", "/")[:70] for x in v["samples"])
            md.append(f"- `{kw}` — {v['count']} hits — e.g. {samples}")
    md.append("\n## Mutex/IDM-like identifiers (sample)\n")
    for s in interesting_mutex_like[:25]:
        md.append(f"- {s}")
    with open(os.path.join(outdir, name + ".scan.md"), "w") as f:
        f.write("\n".join(md))

    print(f"[ok] {name}: {len(info['imports'])} DLLs, {sum(len(v) for v in info['imports'].values())} imported funcs, "
          f"{len(ascii_strs)}+{len(utf16_strs)} strings, keyword cats: {list(hits)}")


if __name__ == "__main__":
    main()
