#!/usr/bin/env python3
"""Clean-room strings scanner for ELF binaries (FDM study).
Reads only printable string constants. No disassembly, no decompilation."""
import json
import os
import re
import sys

KEYWORDS = {
    "http_headers": [
        "Range:", "Content-Range", "Accept-Ranges", "Content-Length", "Content-Type",
        "Last-Modified", "If-Modified-Since", "If-Range", "ETag", "Expect:", "Connection",
        "Keep-Alive", "User-Agent", "Authorization", "Cookie", "multipart", "chunked",
    ],
    "protocols": [
        "http://", "https://", "ftp://", "sftp", "ftps://", "torrent", "magnet:", "thunder://",
    ],
    "transfer_core": [
        "segment", "fragment", "chunk", "part", "resume", "restart", "retry", "speed",
        "bandwidth", "connection", "thread", "mirror", "schedule", "queue", "priority",
    ],
    "engine_modules": [
        "downloadswww", "downloadsfragments", "downloadsbatch", "downloadsm3u",
        "downloadsjsp", "downloadstest", "vmscl", "fdm",
    ],
    "storage": ["sqlite", "CREATE TABLE", "INSERT INTO", "download", ".fdm", "wal", "journal"],
}


def strings(data: bytes, min_len=6):
    pat = re.compile(rb"[\x20-\x7e]{%d,}" % min_len)
    return [m.group().decode("ascii", "ignore") for m in pat.finditer(data)]


def main():
    path, outdir = sys.argv[1], sys.argv[2]
    os.makedirs(outdir, exist_ok=True)
    name = os.path.basename(path)
    data = open(path, "rb").read()
    ss = strings(data)
    hits = {}
    for cat, kws in KEYWORDS.items():
        cat_hits = {}
        for kw in kws:
            m = [s for s in ss if kw.lower() in s.lower()]
            if m:
                cat_hits[kw] = {"count": len(m), "samples": m[:8]}
        if cat_hits:
            hits[cat] = cat_hits
    interesting = sorted({s for s in ss if re.search(
        r"(?i)(segment|fragment|resume|range:|chunk|mirror|connection-?per|retry|timeout)",
        s) and len(s) < 100})[:60]
    result = {"file": name, "size": len(data), "strings_total": len(ss),
              "keyword_hits": hits, "transfer_vocab_samples": interesting}
    with open(os.path.join(outdir, name + ".strings.json"), "w") as f:
        json.dump(result, f, indent=1)
    print(f"[ok] {name}: {len(ss)} strings, cats: {list(hits)}")
    for cat, kws in hits.items():
        for kw, v in kws.items():
            print(f"   {cat[:10]:10s} {kw:20s} {v['count']:5d}  e.g. {v['samples'][0][:64]!r}")


if __name__ == "__main__":
    main()
