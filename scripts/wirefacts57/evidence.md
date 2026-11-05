# wirefacts57 — evidence (issue #57, probed live 2026-11-05)

## rclone serve s3 v1.75.2 (anonymous mode, well-formed SigV4 throwaway keys — W12)
```
WF0: server ready on http://127.0.0.1:26946 (anonymous GET)
=== WF1: 64 KiB UploadPart into a live multipart ===
uploadId: a2951e1f-441b-449b-8db9-...
part1(64KiB) -> 200 etag="18a0b420d6601608635...
WF1: PASS (small part accepted)
=== WF2: 4 concurrent part PUTs on the same upload ===
codes: 200 200 200 200
WF2: PASS (K=4 concurrent parts OK)
=== WF3: ListParts mid-flight ===
listed parts: 5
WF3: PASS
=== WF4: Complete with all-small parts (REAL etags) is REJECTED ===
complete(5 small parts, real etags) -> 200  body: <?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult>
  <Location></Location>
  <Bucket>probe57</Bucket>
  <Key>floor.bin</Key>
  <ETag>&#34;ddb9b62cbbe694110e69e68195af4f6c-5&#34;</E
WF4: NOTE — server ACCEPTED small parts (non-standard; probe must still abort)
=== WF5: Abort (fresh upload, never completed) -> 204, then ListParts -> 404 ===
abort -> 204
list-parts-after-abort -> 404
WF5: PASS (no orphan)
=== WF6: full probe-transaction cost (create -> 1x64KiB -> abort), x5 ===
  tx1: 20 ms
  tx2: 19 ms
  tx3: 20 ms
  tx4: 20 ms
  tx5: 20 ms
mean: 19 ms
=== wirefacts57 rclone leg DONE ===
```

## SeaweedFS 4.51 (md5-verified release asset)
```
WF0: server ready on http://127.0.0.1:21031 (anonymous GET)
=== WF1: 64 KiB UploadPart into a live multipart ===
uploadId: 4b21b499791d2dfbc6516606...
part1(64KiB) -> 200 etag="cf0670d346c3c02f2a4...
WF1: PASS (small part accepted)
=== WF2: 4 concurrent part PUTs on the same upload ===
codes: 200 200 200 200
WF2: PASS (K=4 concurrent parts OK)
=== WF3: ListParts mid-flight ===
listed parts: 5
WF3: PASS
=== WF4: Complete with all-small parts (REAL etags) is REJECTED ===
complete(5 small parts, real etags) -> 400  body: <?xml version="1.0" encoding="UTF-8"?>
<Error><Code>EntityTooSmall</Code><Message>Your proposed upload is smaller than the minimum allowed object size.</Message><Resource>/probe57/floor.bin</Resource>
WF4: PASS (the floor/size gate binds at complete -> 400)
=== WF5: Abort (fresh upload, never completed) -> 204, then ListParts -> 404 ===
abort -> 204
list-parts-after-abort -> 404
WF5: PASS (no orphan)
=== WF6: full probe-transaction cost (create -> 1x64KiB -> abort), x5 ===
  tx1: 23 ms
  tx2: 23 ms
  tx3: 23 ms
  tx4: 25 ms
  tx5: 23 ms
mean: 23 ms
=== wirefacts57 seaweedfs leg DONE ===
```
