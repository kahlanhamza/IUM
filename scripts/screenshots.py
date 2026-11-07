#!/usr/bin/env python3
"""Drive the real IUM frontend (vite dev + a Tauri IPC shim) and capture
the product screenshots into download/screenshots/, mirrored to
docs/screenshots/ for the README (#77).

The shim speaks the REAL @tauri-apps/api v2 wire protocol (wire facts read
from node_modules/@tauri-apps/api/{core,event}.js):
  - `invoke(cmd, args)`  → window.__TAURI_INTERNALS__.invoke
  - `listen(event, cb)`  → invoke('plugin:event|listen', {handler: id})
    where id comes from window.__TAURI_INTERNALS__.transformCallback(cb)
  - delivery             → cb({event, id, payload})
The fixtures are journal-shaped (JobRecord/ChunkView/JobSnapshot from
api.ts) — the UI's own contract, so what the shots show is what the
engine's types say a running session looks like.
"""

import json
import os
import shutil

from playwright.sync_api import sync_playwright

BASE = "http://localhost:5173"
OUT = "/home/z/my-project/download/screenshots"
DOCS = "/home/z/my-project/docs/screenshots"
os.makedirs(OUT, exist_ok=True)

ENGINE_SENTENCE = "ium-core v0.1.1 ready — adapters: http_one_shot, s3_multipart, tus"

# The OFFICIAL Tauri mock harness (node_modules/@tauri-apps/api/mocks.cjs),
# inlined into the page by the bootstrap below. Vendor protocol, not a
# hand-rolled one — this is the lesson from the first capture attempt.
MOCKS_CJS = open(
    "/home/z/my-project/ium-app/node_modules/@tauri-apps/api/mocks.cjs",
    encoding="utf-8",
).read()

SHIM = r"""
(() => {
  // ---- load the official mock harness (CommonJS build, self-contained) ----
  const mod = { exports: {} };
  new Function("module", "exports", MOCKS_CJS_SRC)(mod, mod.exports);
  const { mockIPC, mockWindows } = mod.exports;

  const GiB = 1024 ** 3, MiB = 1024 ** 2;

  function chunks(total, parts, confirmedParts, leasedBytes) {
    const size = Math.floor(total / parts), out = [];
    for (let i = 0; i < parts; i++) {
      const begin = i * size, end = i === parts - 1 ? total : begin + size;
      let state = "pending", bytes = 0, token = null;
      if (i < confirmedParts) { state = "verified"; bytes = end - begin; token = "tok-" + i; }
      else if (i === confirmedParts) { state = "leased"; bytes = leasedBytes || 0; token = "tok-live"; }
      out.push({ idx: i, begin, end, state, bytes_confirmed: bytes, adapter_token: token });
    }
    return out;
  }

  const jobs = {
    "job-8f3c": {
      record: {
        id: "job-8f3c",
        source_path: "/home/you/datasets/model-weights-14b.safetensors",
        source_size: 32 * GiB,
        adapter: "tus",
        endpoint: "https://uploads.example.com/files",
        state: "running",
        capability_badge: "resumable",
        created_at: 1794051000,
        updated_at: 1794051062,
      },
      confirmedParts: 6,
      leased: 420 * MiB,
    },
    "job-2d91": {
      record: {
        id: "job-2d91",
        source_path: "/home/you/backups/photos-2026.tar.zst",
        source_size: 4 * GiB,
        adapter: "s3_multipart",
        endpoint: "https://s3.eu-central-1.amazonaws.com/ium-backups/photos-2026.tar.zst",
        state: "paused",
        capability_badge: "resumable",
        created_at: 1794049300,
        updated_at: 1794050101,
      },
      confirmedParts: 10,
      leased: 0,
    },
    "job-77aa": {
      record: {
        id: "job-77aa",
        source_path: "/home/you/videos/render-final.mov",
        source_size: 800 * MiB,
        adapter: "http_oneshot",
        endpoint: "https://relay.example.com/ingest",
        state: "done",
        capability_badge: "one_shot",
        created_at: 1794048000,
        updated_at: 1794048570,
      },
      confirmedParts: 4,
      leased: 0,
    },
    "job-c41e": {
      record: {
        id: "job-c41e",
        source_path: "/home/you/exports/iso-2026.iso",
        source_size: 5 * GiB,
        adapter: "tus",
        endpoint: "https://mirror.example.org/tus",
        state: "failed",
        capability_badge: "resumable",
        created_at: 1794047000,
        updated_at: 1794047500,
      },
      confirmedParts: 3,
      leased: 0,
    },
  };
  const doneChunks = chunks(800 * MiB, 4, 4, 0);
  const snapEvents = () => [
    [1, "job_created", '{"plan":16,"chunk":2147483648}'],
    [2, "link_profile", '{"kind":"PerFlowCap","sk_over_s1":3.9}'],
    [3, "fleet_adjusted", '{"to":4,"why":"goodput"}'],
    [4, "chunk_verified", '{"idx":5}'],
    [5, "lease_granted", '{"idx":6,"conn":1}'],
  ];

  const snapshot = (id) => {
    const j = jobs[id];
    return {
      job: j.record,
      chunks: id === "job-77aa" ? doneChunks : chunks(j.record.source_size, 16, j.confirmedParts, j.leased),
      events: snapEvents(j),
    };
  };

  const schema = [
    {
      adapter: "mega", label: "MEGA (rclone bridge)", note: "Your MEGA login — the engine bridges through rclone.",
      fields: [
        { name: "user", label: "Email", secret: false, required: true },
        { name: "pass", label: "Password", secret: true, required: true },
      ],
    },
    {
      adapter: "terabox", label: "TeraBox", note: "The ndus cookie from a signed-in browser session.",
      fields: [{ name: "ndus", label: "ndus cookie", secret: true, required: true }],
    },
    {
      adapter: "feishu", label: "Feishu Drive", note: "An app access token with drive scope.",
      fields: [{ name: "token", label: "App token", secret: true, required: true }],
    },
  ];
  const views = {
    mega: {
      user: { label: "Email", secret: false, required: true, present: true, value: "you@example.com" },
      pass: { label: "Password", secret: true, required: true, present: true, value: null },
    },
    terabox: {
      ndus: { label: "ndus cookie", secret: true, required: true, present: false, value: null },
    },
    feishu: {
      token: { label: "App token", secret: true, required: true, present: false, value: null },
    },
  };

  // ---- the IPC surface: every #[tauri::command] the shell exposes ----
  mockIPC((cmd, args) => {
    args = args || {};
    switch (cmd) {
      case "engine_status":
        return "ium-core v0.1.1 ready — adapters: http_one_shot, s3_multipart, tus";
      case "list_adapters":
        return ["http_oneshot", "s3_multipart", "tus", "feishu_drive", "terabox", "mega"];
      case "list_jobs":
        return Object.values(jobs).map((j) => j.record);
      case "snapshot":
      case "inspect_chunks":
        return snapshot(args.jobId);
      case "stat_file":
        return 9243979776;
      case "create_job": {
        const id = "job-" + Math.random().toString(16).slice(2, 6);
        const spec = args.spec || {};
        jobs[id] = {
          record: {
            id,
            source_path: spec.source_path || "",
            source_size: spec.source_size || 0,
            adapter: spec.adapter || "tus",
            endpoint: spec.endpoint || "",
            state: "running",
            capability_badge: "resumable",
            created_at: Date.now(),
            updated_at: Date.now(),
          },
          confirmedParts: 0,
          leased: 64 * 1024,
        };
        return jobs[id].record;
      }
      case "pause_job":
        jobs[args.jobId] && (jobs[args.jobId].record.state = "paused");
        return null;
      case "resume_signal":
        jobs[args.jobId] && (jobs[args.jobId].record.state = "running");
        return null;
      case "cancel_job":
        jobs[args.jobId] && (jobs[args.jobId].record.state = "canceled");
        return jobs[args.jobId] ? jobs[args.jobId].record : null;
      case "credentials_schema":
        return schema;
      case "credentials_get":
        return views;
      case "credentials_set":
      case "credentials_delete":
        return null;
      case "telemetry_status":
        return {
          consent: true,
          count: 2,
          path: "/home/you/.local/share/ium/telemetry.jsonl",
        };
      case "telemetry_set_consent":
      case "telemetry_clear":
        return null;
      default:
        console.warn("[ium-shim] unhandled command", cmd);
        return null;
    }
  }, { shouldMockEvents: true });

  mockWindows("main");

  // The live transfer: the shell's speed column reads Δconfirmed/Δt over
  // job_updated events — feed it real deltas through the OFFICIAL emit
  // path (invoke('plugin:event|emit')).
  window.__iumEmit = (event, payload) => {
    window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event, payload });
  };
  window.__iumTick = () => {
    const j = jobs["job-8f3c"];
    if (!j || j.record.state !== "running") return;
    j.leased += 21 * MiB;
    if (j.leased >= 2 * GiB) { j.leased = 96 * MiB; j.confirmedParts = Math.min(j.confirmedParts + 1, 15); }
    window.__iumEmit("job_updated", snapshot("job-8f3c"));
  };
})();
"""

# Embed the vendor mock source as a JS string literal (json.dumps = valid JS).
SHIM = SHIM.replace("MOCKS_CJS_SRC", json.dumps(MOCKS_CJS))


def main() -> None:
    with sync_playwright() as p:
        browser = p.chromium.launch(
            args=["--no-sandbox", "--disable-dev-shm-usage"]
        )

        # ---------------------------------------------------- dark pass
        ctx = browser.new_context(
            viewport={"width": 1440, "height": 900},
            device_scale_factor=2,
            color_scheme="dark",
        )
        pg = ctx.new_page()
        pg.add_init_script(SHIM)
        pg.goto(BASE)
        pg.wait_for_selector("table tbody tr", timeout=20000)
        # 4 live samples -> the speed column carries a real Δ
        for _ in range(4):
            pg.evaluate("window.__iumTick()")
            pg.wait_for_timeout(900)
        pg.wait_for_function(
            "Boolean(document.querySelector('table td:nth-child(6)')?.textContent.match(/\\/s/))",
            timeout=8000,
        )
        pg.screenshot(path=f"{OUT}/01-main-window.png")

        # filled intake: path stat'ed, endpoint typed, protocol picked
        pg.fill("#upload-path", "/home/you/datasets/fec-parity-bench.bin")
        pg.wait_for_selector(".dropzone--filled", timeout=8000)
        pg.select_option("#upload-adapter", "mega")
        pg.fill("#upload-endpoint", "https://mega.nz/fm/ium-weights")
        pg.wait_for_timeout(350)
        pg.screenshot(path=f"{OUT}/02-create-upload-filled.png")

        # inspector drawer on the live job
        pg.click("table tbody tr:nth-child(1)")
        pg.wait_for_selector('div[role="dialog"]', timeout=8000)
        pg.wait_for_timeout(450)
        pg.screenshot(path=f"{OUT}/03-inspector-chunks.png")
        pg.keyboard.press("Escape")
        pg.wait_for_timeout(250)

        # settings: privacy card leads the dialog
        pg.click("#app-settings")
        pg.wait_for_selector('div[aria-label^="Settings"]', timeout=8000)
        pg.wait_for_timeout(600)
        pg.screenshot(path=f"{OUT}/04-settings-privacy.png")

        # scrolled to the credentials vault + About footer
        pg.eval_on_selector(
            'div[aria-label^="Settings"]',
            "el => el.scrollTo(0, el.scrollHeight)",
        )
        pg.wait_for_timeout(350)
        pg.screenshot(path=f"{OUT}/05-settings-credentials.png")
        ctx.close()

        # ---------------------------------------------------- light pass
        ctx2 = browser.new_context(
            viewport={"width": 1440, "height": 900},
            device_scale_factor=2,
            color_scheme="light",
        )
        pg2 = ctx2.new_page()
        pg2.add_init_script(SHIM)
        pg2.goto(BASE)
        pg2.wait_for_selector("table tbody tr", timeout=20000)
        for _ in range(4):
            pg2.evaluate("window.__iumTick()")
            pg2.wait_for_timeout(900)
        pg2.wait_for_function(
            "Boolean(document.querySelector('table td:nth-child(6)')?.textContent.match(/\\/s/))",
            timeout=8000,
        )
        pg2.screenshot(path=f"{OUT}/06-main-window-light.png")
        ctx2.close()

        browser.close()

    os.makedirs(DOCS, exist_ok=True)
    for f in sorted(os.listdir(OUT)):
        if f.endswith(".png"):
            shutil.copyfile(f"{OUT}/{f}", f"{DOCS}/{f}")
    print("shots written:", sorted(os.listdir(OUT)))


if __name__ == "__main__":
    main()
