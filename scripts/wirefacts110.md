## LIVE REHEARSAL GREEN — the close condition is met: device-code grant redeemed, full upload chain exercised on the real Graph wire, quickXorHash verified local == commit receipt == metadata read-back

The device-code pivot paid off. One owner approval at `login.microsoft.com/device` (code `CQ2HD8V58`, redeemed with 667 s still on the 900 s window), and the driver (`scripts/onedrive_rehearsal.py --wait`) ran the entire rehearsal autonomously — no redirect URI, no port, no paste race, the exact failure mode that killed both authorization-code attempts (AADSTS70008 ×2) cannot recur in this shape.

**Wire facts (masked; token lengths only, no PII, drive id truncated):**

- **Grant:** device-code poll → `200` on the token endpoint. access len=2614 `eyJ0eX…`, refresh len=1477 `1.AToA…`, expires_in=4019 s. Granted scope echo: `Files.ReadWrite User.Read profile openid email` — note: `offline_access` is NOT echoed (Entra omits it from the echo) yet the refresh token WAS issued; the driver asserts RT presence as the truth, the echo is cosmetic. Saved 0600.
- **probe:** `GET /v1.0/me/drive` → `200`, `driveType=business`, driveId `b!fio9OM…8aW`, quota `total=107374182400 used=0 state=normal`.
- **createUploadSession** (conflictBehavior=replace) → `200`, expiry `2026-11-14T13:57:21.313Z`, uploadUrl stashed as the resumable handle.
- **Fragment plan** 11,534,459 bytes total: `[0, 5242880)` + `[5242880, 10485760)` + tail `[10485760, 11534459)` — every non-final fragment a 320 KiB multiple (5 MiB = 16 × 327,680), tail ≤ 60 MiB. Asserted in-code before any PUT.
- **chunk0** `PUT Content-Range bytes 0-5242879/11534459` → `202`, `nextExpectedRanges=['5242880-11534458']`.
- **DELIBERATE misalignment** (claimed start 6291456, one MiB ahead) → **`416`** — and the wire nuance worth banking: **the 416 body carried `nextExpectedRanges=[]`** (empty). The server refuses without necessarily restating the range in the error body.
- **That empty-body 416 VALIDATES the landed adapter's design**: `ium_core::onedrive::session_next_byte()` (src/onedrive.rs:754) does not trust the 416 body at all — it GETs the self-authenticated `uploadUrl` (the session IS the resumable handle) and reads `nextExpectedRanges` from the session's own answer; a dead session answers 404 → honest restart path. Server wins, exactly the #10 realign pattern.
- **chunk1 realigned** `[5242880,10485760)` → `202`, `nextExpectedRanges=['10485760-11534458']`.
- **final PUT** `[10485760,11534459)` → **`201`** — the atomic commit itself, no separate finish call; the driveItem comes back in the same response: `size=11534459`, `quickXorHash=UzXHka6L0zPIU07xNLV0YCWpuMc=`.
- **verify-before-declare (ADR-004):** metadata read-back `GET …?$select=id,name,size,file` → `200`, same `quickXorHash`; LOCAL hash (independent Python implementation, the 12-vector cross-checked one) → same value. **TRIPLE MATCH: local == commit receipt == metadata read-back.** The `sha256Hash` Graph offers was never touched, per the standing distrust.
- **cleanup** `DELETE` → `204`. The rehearsal object is gone; the drive is back to `used=0`.

**Ledger:** 13 rows appended to `.secrets/onedrive_rehearsal_ledger.txt` (LOCAL ONLY, 0600 — the secrets discipline holds: nothing identifying, nothing token-shaped, crossed the wire only in masked form).

**Token-loss rehearsal note:** this session also proved the refresh-token story on a real RT — rotation-on-use write-through is the adapter's #81 pattern and the grant's RT (1477 chars) is on disk 0600 for the adapter-path reuse.

Close condition per the design note: **probe → createUploadSession → fragments → deliberate 416 → realign → atomic final PUT → quickXorHash triple-match → cleanup — all GREEN on the real wire.** The implementation (06968b0, CI 43255641309 8/8) + this rehearsal = the lane's definition of done. Gates then push then verdict, per the loop.