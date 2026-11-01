# Clean-room static scan: `idman643build16.exe`

- Size: 12,698,904 bytes | Machine: 0x14c (0x14c=i386, 0x8664=x64) | Subsystem: 2 (2=GUI, 3=CLI) | .NET: False
- Linker: 9.0 | Entry: 0x5b7a | Exports: 0

## Sections

| Section | Virtual size | Raw size | Entropy | Note |
|---|---|---|---|---|
| .text | 46,759 | 47,104 | 6.623 |  |
| .rdata | 14,026 | 14,336 | 5.282 |  |
| .data | 11,964 | 8,192 | 2.65 |  |
| .rsrc | 18,264 | 18,432 | 4.504 |  |
| .reloc | 5,714 | 6,144 | 4.036 |  |

## Import table (DLLs and function counts)

| DLL | Functions | Transfer-relevant |
|---|---|---|
| ADVAPI32.dll | 4 |  |
| KERNEL32.dll | 84 |  |
| SHELL32.dll | 2 |  |
| USER32.dll | 14 |  |

## Keyword evidence (strings)


### protocols

- `http://` — 18 hits — e.g. http://ocsp.digicert.com0A; 5http://cacerts.digicert.com/DigiCertTrustedRootG4.crt0C; 2http://crl3.digicert.com/DigiCertTrustedRootG4.crl0; http://www.digicert.com/CPS0; Mhttp://crl3.digicert.com/DigiCertTrustedG4CodeSigningRSA4096SHA384202; Mhttp://crl4.digicert.com/DigiCertTrustedG4CodeSigningRSA4096SHA384202

### transfer_core

- `thread` — 7 hits — e.g. - unexpected multithread lock error; - not enough space for thread data; `local static thread guard'; CreateThread; GetExitCodeThread; ExitThread
- `download` — 6 hits — e.g. Internet Download Manager; <description>Internet Download Manager installer</description>; Please visit http://www.internetdownloadmanager.com; Internet Download Manager installer; Internet Download Manager (IDM); Internet Download Manager installer

### file_io

- `SetFilePointer` — 1 hits — e.g. SetFilePointer
- `WriteFile` — 1 hits — e.g. WriteFile
- `CreateFile` — 2 hits — e.g. CreateFileW; CreateFileMappingA
- `CreateFileMapping` — 1 hits — e.g. CreateFileMappingA
- `MapViewOfFile` — 2 hits — e.g. MapViewOfFile; UnmapViewOfFile

## Mutex/IDM-like identifiers (sample)

-    name="Tonec.IDM.idman643build16"
- IDM Setup
- IDM requires Windows 7 or later.
- IDM1.tmp
- IDMFTYPE
- IDM_Setup_Mutex_st23nZ7a
- IDM_Setup_Temp
- Initializing IDM Installation Wizard...
- Internet Download Manager (IDM)
- components2/idmcchandler2_64.dllUT