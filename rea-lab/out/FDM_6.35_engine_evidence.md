# REA Evidence Dossier — Free Download Manager 6.35 (Linux build)

**Target:** FDM 6.35.2 · **Acquired:** 2026-11-01 · **Method:** clean-room static analysis
(installer unpack + string/symbol extraction only — no disassembly, no decompilation, no code reproduction)

## Acquisition

| Artifact | Size | Result |
|---|---|---|
| `fdm_x64_setup.exe` (Windows, from files2.freedownloadmanager.org/6/latest) | 52,144,296 B | Custom container; 7z 25.01 cannot parse inner payload (gzip magic hits are false positives / custom flags) |
| `freedownloadmanager.deb` (Linux, same 6/latest channel) | 42 MB | **Clean unpack** via `ar x` + `tar` |
| `fdm.dmg` (macOS) | — | not needed, Linux build sufficient |

## Correction to prior assumption

FDM 6.x is **not Electron**. No `.asar` exists anywhere in the package.
FDM 6 = **Qt6/QML frontend + native C++ engine built as one plugin module per protocol family.**

## Engine layout (`/opt/freedownloadmanager/`)

```
fdm                     ← main binary (Qt Quick/QML UI shell)
wenativehost            ← browser-extension native-messaging host ("WE" = web extension)
qml/, plugins/          ← Qt6 runtime (Quick, Wayland/XCB platforms, TLS backends)
lib/libdownloadswww.so.6.35      ← HTTP(S)/FTP core transfer module
lib/libdownloadsfragments.so.6.35← segmentation engine
lib/libdownloadsbt.so.6.35       ← BitTorrent module
lib/libdownloadsb/ms/jsp/m3u/batch/test .so*  ← browser-media / HLS-ish / json-post / m3u / batch / self-test modules
lib/libvmsclshared.so.6.35.2     ← shared core library (VMSCL)
plugins/sqldrivers/libqsqlite.so ← SQLite persistence driver
translations/ (85 locales)       ← heavy i18n investment
```

## Class inventory (demangled C++ symbol names only)

### `libdownloadsfragments` (31 app classes, 1,080 mangled symbols)
`AbstractDownload, AbstractDownloadFileInfo, AbstractDownloadFileOps, AbstractDownloadModule,
AbstractDownloadModuleEntry, AbstractDownloadPersistentInfo, AbstractDownloadResourceDescription,
AbstractDownloadRunTimeInfo, AbstractDownloadSection, AbstractDownloadSourceInfo,
AbstractDownloadsModulesProvider, AbstractLiveDownloadRunTimeInfo, AbstractLockedDownloadOperationInfo,
FragmentDownloadsModule, FragmentsDownload, IndependentSegmentsDownloader, DownloadableSubtitlesInfo …`

Key callsites observed in symbol table:
- `IndependentSegmentsDownloader::setWritersThread(QThread*)` — **dedicated writer thread decoupled from network workers**
- `QHash<QtHostAndPort, QtConnection…>` — **per-host connection accounting**
- `QArrayDataPointer<AbstractDownloadMirrorInfo>` — **mirror descriptor lists**
- `DownloadErrorAllowedToAutoRetry` — **error→auto-retry policy map**
- `toDownloadScheduleRecord(QHash<QString, QVariant>)` — **scheduler records**

### `libdownloadswww` (56 app classes, 1,772 mangled symbols)
`AbstractWwwDownloader, WwwDownloader, WwwDownload, WwwDownloadSection, CurlDownloader,
DownloadMirrorsUsageManager, FileDataAsyncWriter, MemoryDataAsyncWriter, AbstractDataAsyncProcessor,
AbstractNetworkConsumer, AbstractNetworkConsumersPack, AbstractNetworkConsumersStats,
AbstractNetworkConsumerWithExtraTasks, AbstractNetworkConsumersWithExtraTasksPack,
HttpHeadersParser, AbstractFileTypeDetector, AbstractError, AbstractDownloadPersistentInfo …`

### `libdownloadsbt` (49 app classes, 5,098 mangled symbols)
`BtDownload, BtDownloadsModule, BtDownloadFileOps, BtTrackerList, AbstractNetworkAccessChief,
OperationSpeedLogger, ShortBlobUrlsManager, MediaPlayAsapUtils …`

## HTTP resume vocabulary (raw strings in `libdownloadswww`)

| Token | Count | Significance |
|---|---|---|
| `Content-Range` | 5 | byte-range response parsing |
| `Accept-Ranges` | 2 | server capability probe |
| `Range` | 27 | request construction |
| `WwwDownloadSection::compareDownloadsByHttpEtag` | 1 | **ETag-based resume validation before reattach** |
| `Retry-After:` | 1 | 429/503 backoff honored |
| `Expect: 100-continue` | 1 | avoid dead POSTs through proxies |
| `TLS_client_method`, `Failed set SNI` | 27/1 | OpenSSL transport |
| `Lying server, not serving HTTP/2` | 1 | HTTP/2 capability sniff + fallback |
| `NTLMSSP`, PAC strings | 13/128 | enterprise proxy support |

## Architecture interpretation (10 teachings relevant to IUM)

1. **Plugin module registry** — one `AbstractDownloadModule` per protocol family, discovered via `AbstractDownloadsModulesProvider`. IUM equivalent: `TransferAdapter` trait registry.
2. **Segment = "Section"** — `AbstractDownloadSection` + `IndependentSegmentsDownloader` confirm the segment-pool model (aria2 SegmentMan parity).
3. **Writer/network decoupling** — network consumers produce buffers; a dedicated `QThread` writer commits to disk (FDM's answer to torn writes at offsets).
4. **Per-host connection accounting** — `QHash<QtHostAndPort, …>` prevents one host monopolizing the pool (aria2 `max-connection-per-server` parity).
5. **Mirrors as first-class citizens** — `DownloadMirrorsUsageManager` + `AbstractDownloadMirrorInfo`.
6. **Resume validated by ETag, not assumed** — `compareDownloadsByHttpEtag` before reattaching to old byte offsets.
7. **Policy-driven retries** — `DownloadErrorAllowedToAutoRetry` map instead of hardcoded retry logic.
8. **Persistence layer behind abstract info objects** — `AbstractDownloadPersistentInfo` over SQLite (QSqlite driver present).
9. **Self-test module shipped** — `libdownloadstest.so` (a commercial vendor ships its transfer regression harness inside the product).
10. **i18n from day one** — 85 locale files; community distribution requires it.

## Raw artifacts

- `libdownloadsfragments.so.6.35.strings.json` · `libdownloadswww.so.6.35.strings.json` (keyword scan)
- Source tree kept at `rea-lab/targets/fdm_data/` (gitignored — redistributable? no; only our notes are committed)

## Clean-room statement

Findings above are derived from file names, directory layout, demangled symbol **names**, and string
constants. No proprietary code was copied, decompiled, or reconstructed. This dossier informs an
independent specification (IUM) that does not reproduce FDM code.
