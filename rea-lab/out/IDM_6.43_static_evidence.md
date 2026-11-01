# REA Evidence Dossier — Internet Download Manager 6.43 build 16 (static)

**Target:** IDM 6.43.16.1 (Tonec Inc.) · **Acquired:** 2026-11-01 · **Method:** clean-room static scan
(PE headers, import tables, string constants only — **no decompilation, no disassembly, no unpacking**)

## Acquisition

| Artifact | Size | Source |
|---|---|---|
| `idman643build16.exe` | 12,698,904 B | `https://download.internetdownloadmanager.com/idman643build16.exe` (linked from official download page) |

## Outer container findings (`idman643build16.exe`)

**PE32 installer stub, internally `installer.exe` v6.43.16.1, Tonec Inc. — not packed:**

| Section | Entropy | Note |
|---|---|---|
| .text | 6.623 | normal code |
| .rdata | 5.282 | normal |
| .data | 2.650 | normal |
| .rsrc | 4.504 | resources |
| .reloc | 4.033 | relocations |

- Imports (4 DLLs / 104 functions): `ADVAPI32` (registry), `USER32` (14), `SHELL32` (`SHBrowseForFolderW` — install-path picker), `KERNEL32` (84)
- Payload handling style: `CreateFileMappingA` + `MapViewOfFile` + `SetFilePointer` + `WriteFile`
  (memory-mapped staging of the payload — classic self-installer pattern)
- Signature chain: DigiCert OCSP/CRL strings present; `SECURITY` data directory populated (Authenticode)
- Build stamp: PE timestamp 2026-10-22 — current build, actively shipped

## Inner payload findings

- 7-Zip 25.01 **cannot** parse the container as NSIS (`Cannot open the file as [Nsis] archive`).
- One embedded ZIP stream found (offset ≈ 4,745,895; ~186 KB) containing
  `components2/idmcchandler2_64.dll` (461,848 B — Chrome/Edge browser-integration component).
  The stream carries a data error; the extracted copy has no readable import table (truncated stream),
  so the browser component yields no further evidence.
- Remaining ~7.6 MB tail: opaque, custom-compressed payload holding the real engine
  (`idman.exe`, helper service, localization, skins).

## Verdict

**The IDM core engine (`idman.exe`) is not recoverable with generic static tooling.**
Tonec ships a proprietary installer container; going deeper would require dynamic execution /
unpacking, which is out of scope for this study's clean-room rules and EULA constraints.

**Documented as a finding in itself:** a commercial manager protects its transfer core behind a
custom container + Authenticode-signed stub — the same "moat" posture FDM does *not* take
(FDM's Linux build exposes its whole module architecture via symbol names).

## What still feeds IUM

1. **Confirmation of threat model maturity** — IDM has shipped for 20+ years with registry-based
   licensing (ADVAPI32 imports), a service helper architecture, and per-browser integration
   components (`idmcchandler` naming pattern: `cc` = Chrome? + handler per component slot).
2. **Installer UX bar** — one-file bootstrap, DigiCert signing, memory-mapped install: the
   distribution polish users expect from a "professional" manager.
3. **Negative evidence for architecture claims** — any public claim about IDM's internals
   (e.g., "up to 32 connections, WinInet + raw sockets dual stack") stays OUT of our spec unless
   reproduced from our own clean-room observation. IUM's segmented-core design is justified by
   **aria2 / FDM / XDM / Motrix evidence instead.**

## Raw artifacts

- `idman643build16.exe.scan.json` / `.scan.md` — full header/import/string scan
- `idmcchandler2_64.dll.scan.json` / `.scan.md` — truncated-component scan (kept for the record)

## Clean-room statement

Derived from PE metadata, import/export tables, and string constants of a lawfully downloaded
trial installer. No decompilation, disassembly, unpacking, or code reproduction. Used solely to
inform an independent specification.
