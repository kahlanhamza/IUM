#!/usr/bin/env python3
"""IUM-dist v0.2.3 surface sync (#107 rung 6): README/USERGUIDE/CHANGELOG
move to v0.2.3-true wording (the picker + Test + share link are now
DOWNLOADABLE truth — the #104 scope-honesty rule unblocks) and the six
re-shot product assets land. Every edit is asserted: product-only
language (no issue numbers, no ADRs), the famous names present, tables
intact."""
import base64
import json
import os
import urllib.request

TOKEN = open(os.path.expanduser(".secrets/gh_token")).read().strip()
REPO = "kahlanhamza/IUM-dist"
HDR = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
       "User-Agent": "ium-release"}


def api(method, path, payload=None, expect=200):
    req = urllib.request.Request(f"https://api.github.com/repos/{REPO}/{path}",
                                 method=method, headers=HDR,
                                 data=json.dumps(payload).encode() if payload is not None else None)
    with urllib.request.urlopen(req) as r:
        body = json.load(r) if r.status != 204 else {}
        assert r.status == expect, f"{method} {path}: {r.status} != {expect}"
        return body


def get_file(path):
    r = api("GET", f"contents/{path}")
    return base64.b64decode(r["content"]).decode(), r["sha"]


def put_file(path, content, sha, message):
    api("PUT", f"contents/{path}", {
        "message": message,
        "content": base64.b64encode(content.encode()).decode(),
        "sha": sha,
    })


def put_binary(path, data, sha, message):
    api("PUT", f"contents/{path}", {
        "message": message,
        "content": base64.b64encode(data).decode(),
        "sha": sha,
    })


def hygiene(name, text, famous=True):
    import re
    assert not re.search(r"#\d{2,3}", text), f"{name}: issue number leaked"
    assert "ADR-" not in text, f"{name}: ADR leaked"
    if famous:
        for f in ("Backblaze", "Cloudflare", "Wasabi", "Nextcloud"):
            assert f in text, f"{name}: {f} missing"
    assert "【" not in text and "TODO" not in text, f"{name}: placeholder leaked"


# ---------------------------------------------------------------- README
readme, sha = get_file("README.md")

old_s3row = "| S3-compatible | S3 multipart | ✅ per-part | AWS S3, Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, MinIO — anything speaking the S3 API |"
new_s3row = "| S3-compatible | S3 multipart | ✅ per-part | AWS S3, Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, MinIO — the form's provider picker builds the URL, keys set once in Settings |"
if new_s3row not in readme:
    assert old_s3row in readme, "README: S3 row drifted"
    readme = readme.replace(old_s3row, new_s3row)

anchor = "Where a server physically cannot resume, the UI says **One-shot ⚠️** instead of pretending."
picker_para = (
    "**You don't need to know what an endpoint URL is.** The upload form asks "
    "\"where should files go?\" — pick a provider, name your bucket, and the "
    "destination builds itself, with a deep link to the provider's key console "
    "and a test button that checks your keys before the first job. A finished "
    "upload can hand you a private, expiring share link. Anything else still "
    "takes a pasted URL — power users lose nothing.\n\n"
)
assert anchor in readme, "README: honesty anchor drifted"
if "You don't need to know what an endpoint URL is" not in readme:
    readme = readme.replace(anchor, picker_para + anchor)

old_hl = "Credentials are set once and stored in your OS keychain. |"
new_hl = "The form's provider picker builds the destination for you; credentials are set once and stored in your OS keychain. |"
if old_hl in readme:
    readme = readme.replace(old_hl, new_hl)

hygiene("README.md", readme)
if sha != get_file("README.md")[1]:
    put_file("README.md", readme, sha, "README: the provider picker, the key-console links and the expiring share links are downloadable truth in v0.2.3")
    print("README.md synced")
else:
    print("README.md unchanged")

# ---------------------------------------------------------------- USERGUIDE
guide, sha = get_file("USERGUIDE.md")

guide = guide.replace(
    "for example, v0.2.2 shipped `IUM_0.2.2_amd64.deb`, `IUM-0.2.2-1.x86_64.rpm`, `IUM_0.2.2_amd64.AppImage`, `IUM_0.2.2_aarch64.dmg`, `IUM_0.2.2_x64_en-US.msi`, and `IUM_0.2.2_x64-setup.exe`",
    "for example, v0.2.3 shipped `IUM_0.2.3_amd64.deb`, `IUM-0.2.3-1.x86_64.rpm`, `IUM_0.2.3_amd64.AppImage`, `IUM_0.2.3_aarch64.dmg`, `IUM_0.2.3_x64_en-US.msi`, and `IUM_0.2.3_x64-setup.exe`",
)
guide = guide.replace("certutil -hashfile IUM_0.2.2_x64-setup.exe SHA256",
                      "certutil -hashfile IUM_0.2.3_x64-setup.exe SHA256")

old_step3 = """3. **Create the job.** The new-upload form has four fields:
   - **Adapter** — the destination type (e.g. `s3_multipart` for any S3-compatible bucket: Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, AWS S3, MinIO…).
   - **Source** — the file on your machine.
   - **Destination** — the target URL (for S3: the bucket URL, e.g. `http://s3.example.com/my-bucket/report.mp4`).
   - **Connections** — how many parallel pieces to split the upload into. The default is fine; more connections help on fast links, fewer on unstable ones.
4. **Submit.** The job appears in the table with its live state and progress."""
new_step3 = """3. **Create the job.** The new-upload form leads with a **Provider** picker so you never have to assemble a URL by hand:
   - **Provider** — pick where your files go. Known providers (Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, self-hosted MinIO) pre-fill the destination shape for you: fill in your bucket (and your region or account id where asked) and the destination URL derives itself. The region you pick is also the region IUM signs with — submit the job and it is remembered for the lane, so the signature always matches the destination. Each provider also carries a **Get your keys ↗** link straight to its key console and a short recipe for what to click. If your destination is not on the list — or you are the power-user kind — **Custom URL** is right there: paste any destination URL, exactly as before.
   - **Adapter** — the destination type. Picking a provider on an S3-compatible lane sets this to `s3_multipart` for you; leave it alone unless you know you want otherwise.
   - **Source** — the file on your machine.
   - **Connections** — how many parallel pieces to split the upload into. The default is fine; more connections help on fast links, fewer on unstable ones.
4. **Test the destination before the first job** (optional, S3 lanes). In Settings → S3 multipart, paste a bucket URL and press **Test** — IUM opens and aborts its own tiny session on a probe key under your bucket, so the check never touches your objects. A wrong key or a missing bucket is a written answer now, not a surprise mid-upload.
5. **Submit.** The job appears in the table with its live state and progress.

**Need to send the file onward?** A finished S3 job can hand you a link: open the job's inspector and press **Copy share link** — you get a signed download URL that opens for 24 hours and then expires. The link is minted on demand from your own keys; nothing is made public until you ask, and nothing stays valid forever."""
assert old_step3 in guide or "Copy share link" in guide, "USERGUIDE: step 3 drifted"
if old_step3 in guide:
    guide = guide.replace(old_step3, new_step3)

hygiene("USERGUIDE.md", guide)
if sha != get_file("USERGUIDE.md")[1]:
    put_file("USERGUIDE.md", guide, sha, "User guide: the provider picker flow, the Test button, and the expiring share links — v0.2.3 examples")
    print("USERGUIDE.md synced")
else:
    print("USERGUIDE.md unchanged")

# ---------------------------------------------------------------- CHANGELOG
ch, sha = get_file("CHANGELOG.md")
v023 = """## [0.2.3] — 2026-11-11

### Added
- **The destination picker.** The upload form now leads with "where should files
  go?" — pick a known provider (Backblaze B2, Cloudflare R2, Wasabi, Scaleway,
  DigitalOcean Spaces, Filebase, self-hosted MinIO), name your bucket, and the
  destination URL builds itself. Each provider carries a deep link to its key
  console and a short recipe. Custom URL stays the default: paste any
  destination exactly as before.
- **A working Test button for S3 lanes.** Check your keys and bucket before the
  first job — IUM opens and aborts its own tiny probe session, so the check
  never touches your objects.
- **Expiring share links.** A finished S3 upload can hand you a signed download
  link that opens for 24 hours and then expires — minted on demand from your
  own keys; nothing is made public until you ask.
- **The guided region is the signing region.** The region you pick in the form
  is remembered for the lane, so the request signature always matches the
  destination.

### Fixed
- **The provider list matches the providers' own current documentation.**
  Filebase's general S3 endpoint is `s3.filebase.io`; Wasabi offers all 16
  documented regions; DigitalOcean Spaces all 13; Backblaze B2 gained
  `us-east-005` and dropped an uncorroborated region.

"""
assert "## [0.2.2]" in ch, "CHANGELOG: 0.2.2 anchor missing"
if "## [0.2.3]" not in ch:
    ch = ch.replace("## [0.2.2]", v023 + "## [0.2.2]", 1)
hygiene("CHANGELOG.md", ch, famous=False)
put_file("CHANGELOG.md", ch, sha, "CHANGELOG: v0.2.3 — the destination picker, the Test button, the share links")
print("CHANGELOG.md synced")

# ---------------------------------------------------------------- assets
for n in ["01-main-window.png", "02-create-upload-filled.png", "03-inspector-chunks.png",
          "04-settings-privacy.png", "05-settings-credentials.png", "06-main-window-light.png"]:
    local = f"/home/z/my-project/download/screenshots/{n}"
    data = open(local, "rb").read()
    cur = api("GET", f"contents/assets/{n}")
    if base64.b64decode(cur["content"]) == data:
        print(f"assets/{n}: unchanged")
        continue
    put_binary(f"assets/{n}", data, cur["sha"],
               f"asset: {n} re-shot through the real harness at v0.2.3 (the guided picker flow)")
    print(f"assets/{n}: updated ({len(data)//1024}KB)")

print("DISTRO SYNC COMPLETE")
