# IUM — User Guide

Everything you need to go from download to a completed upload. Every command and path in this guide is real — they are the ones IUM actually uses.

---

## 1. Download

All releases live on the [Releases page](https://github.com/kahlanhamza/IUM-dist/releases). Each release ships:

| Platform | Files |
|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` |
| macOS (Apple Silicon) | `.dmg` |
| Windows | `.msi` (installer), `.exe` (setup wizard) |

A current release ships exactly these installers — for example, v0.2.0 shipped `IUM_0.2.0_amd64.deb`, `IUM-0.2.0-1.x86_64.rpm`, `IUM_0.2.0_amd64.AppImage`, `IUM_0.2.0_aarch64.dmg`, `IUM_0.2.0_x64_en-US.msi`, and `IUM_0.2.0_x64-setup.exe`.

Alongside the installers you will find `SHA256SUMS.txt` (checksums for every file), a signed attestation (`.attestation.1.jsonl`) and software bill of materials (SBOM) for supply-chain auditing, and `latest.json` (what the built-in updater reads).

## 2. Verify your download

Checksum verification takes ten seconds and rules out a corrupted or tampered download.

**Linux**

```bash
sha256sum -c SHA256SUMS.txt        # run in the folder holding the files
```

**macOS**

```bash
shasum -a 256 -c SHA256SUMS.txt
```

**Windows (PowerShell or Command Prompt)**

```bat
certutil -hashfile IUM_0.2.0_x64-setup.exe SHA256
```

…then compare the printed hash with the matching line in `SHA256SUMS.txt`. Teams that need a supply-chain-grade audit can additionally verify each artifact's signed attestation sidecar against its SBOM.

## 3. Install

**Linux — .deb / .rpm**: install with your package manager (`sudo apt install ./IUM_*.deb` or `sudo dnf install ./IUM-*.rpm`). The packages declare their runtime dependencies (the GTK and WebKitGTK libraries) and your package manager resolves them.

**Linux — .AppImage**: make it executable and run it — it carries its own runtime libraries:

```bash
chmod +x IUM_*.AppImage
./IUM_*.AppImage
```

**macOS**: open the `.dmg` and drag IUM to Applications. See §4 for the first-launch note on Gatekeeper.

**Windows**: run the `.msi` or the `.exe` setup. See §4 for the SmartScreen note.

IUM has no separate service or daemon to install or start — the upload engine is built into the app itself.

## 4. First launch: what the security warnings mean

IUM's installers are **not code-signed yet**. That is a deliberate, known hold — not a hidden problem — and it means the operating systems ask for explicit confirmation on first run:

- **Windows SmartScreen**: if the blue "Windows protected your PC" dialog appears, click **More info** → **Run anyway**. SmartScreen shows this for any installer without a signing certificate.
- **macOS Gatekeeper**: if macOS says the app "can't be opened because Apple cannot check it for malicious software", right-click (or Control-click) the app in Applications → **Open** → **Open** in the dialog. This is needed once; afterwards the app opens normally. The terminal-inclined equivalent: `xattr -d com.apple.quarantine /Applications/IUM.app`.
- **Linux**: nothing to do — no warning gate exists.

## 5. First upload

1. **Launch IUM.** A splash shows the boot state in one live sentence — it reports the real state, and the main window appears either way.
2. **Give the destination its credentials, once.** Open **Settings** and pick the lane you are using (S3-compatible storage, WebDAV, Google Drive, YouTube, Terabox, Feishu, Mega — each appears once its credentials are configured; lanes without credentials simply do not appear, nothing is half-broken). HTTP and TUS destinations need no credentials.
3. **Create the job.** The new-upload form leads with a **Provider** picker so you never have to assemble a URL by hand:
   - **Provider** — pick where your files go. Known providers (Backblaze B2, Cloudflare R2, Wasabi, Scaleway, DigitalOcean Spaces, Filebase, self-hosted MinIO) pre-fill the destination shape for you: fill in your bucket (and your region or account id where asked) and the destination URL derives itself. Each provider also carries a **Get your keys ↗** link straight to its key console and a short recipe for what to click. If your destination is not on the list — or you are the power-user kind — **Custom URL** is right there: paste any destination URL, exactly as before.
   - **Adapter** — the destination type. Picking a provider on an S3-compatible lane sets this to `s3_multipart` for you; leave it alone unless you know you want otherwise.
   - **Source** — the file on your machine.
   - **Connections** — how many parallel pieces to split the upload into. The default is fine; more connections help on fast links, fewer on unstable ones.
4. **Test the destination before the first job** (optional, S3 lanes). In Settings → S3 multipart, paste a bucket URL and press **Test** — IUM opens and aborts its own tiny session on a probe key under your bucket, so the check never touches your objects. A wrong key or a missing bucket is a written answer now, not a surprise mid-upload.
5. **Submit.** The job appears in the table with its live state and progress.

**Need to send the file onward?** A finished S3 job can hand you a link: open the job's inspector and press **Copy share link** — you get a signed download URL that opens for 24 hours and then expires. The link is minted on demand from your own keys; nothing is made public until you ask, and nothing stays valid forever.

**Job states, honestly named:** `created` (queued) → `running` → `done`. A job can sit at `paused` (you paused it), `canceled` (you canceled it), or `failed` — and `failed` always comes with a written reason: open the job's inspector to read what actually happened, taken from the upload journal.

**Closing the lid mid-upload is fine.** The engine journals every confirmed byte. On the next run the upload resumes from where the journal says it stopped — already-confirmed bytes are never re-sent, and the interruption itself is recorded as counted, retryable work rather than silently swallowed.

## 6. When something fails

- **Read the reason.** Every failure carries its own words. Select the failed job → the inspector drawer shows the journal's error events in order. Nothing is paraphrased away.
- **Retry.** Re-submitting the same job resumes from the journal's confirmed position when the destination still holds the partial upload — and from the best honest position otherwise.
- **Diagnostics.** Settings → Diagnostics shows what the app has recorded locally, with a two-step clear if you want a blank slate.
- **Logs for a support conversation.** The session journal lives at:
  - Linux: `~/.local/share/app.ium.desktop/ium-session.db`
  - macOS: `~/Library/Application Support/app.ium.desktop/ium-session.db`
  - Windows: `%APPDATA%\app.ium.desktop\ium-session.db`

## 7. Settings worth knowing

- **Speed cap** — a global upload speed limit (in KB/s) so IUM stays polite to everything else on your network. Turn it off to use the full line.
- **Notifications** — the OS notification on completion and failure, only when you're away from the window. Disable in Settings.
- **Telemetry** — a local, capped, one-click-clearable record of anonymous usage counters. It is collected by default and switched off in Settings → Diagnostics. **Nothing is transmitted in this build — there is no sending code to switch on.** Uploads go only to the destination you chose; the journal never leaves your machine.
- **Credentials vault** — destination credentials are stored in IUM's vault on this machine (under the OS config directory), never inside the journal.

## 8. Updates

IUM checks the release feed (`latest.json`) and offers updates through its built-in updater — on Windows the update applies passively. To update manually: download the new installer, verify it (§2), and install over the old version; your journal, credentials, and preferences carry over.

**Updating from v0.2.1 or earlier — one final manual update.** Those builds check a feed URL on the source repository, which has since gone private, so their update check can no longer reach it. Download v0.2.2 (or newer) from the Releases page once, verify, and install it — from that version on, the update feed lives in a public place and auto-update works permanently.

## 9. Quick troubleshooting

| Symptom | First check |
|---|---|
| Destination lane missing from the form | Its credentials aren't set yet — Settings → credentials for that lane |
| `failed` right at job start | Read the inspector's first error — usually a wrong destination URL or a rejected credential |
| Upload stalls on a laptop that slept | Just resume/re-submit — the journal resumes from confirmed bytes |
| SmartScreen/Gatekeeper blocks first launch | §4 — expected for unsigned installers |
| Update doesn't appear | Compare your version with the latest release; update manually if needed |
