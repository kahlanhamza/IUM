/**
 * The auto-update wrapper (#80): the release bar's "auto-update default-on"
 * made real. One module owns the whole update story so the surface stays
 * testable and the rules stay in one place:
 *
 * - CHECK is automatic and silent; INSTALL is user-armed (the consent rule —
 *   nothing downloads or applies without a click, the Sparkle/Squirrel school);
 * - the degrade rule (#67): outside a real Tauri context every call resolves
 *   to the honest "no updater here" answer instead of throwing — the browser,
 *   vitest and the e2e shim never see a phantom updater;
 * - progress rides a plain {downloaded, total?} callback — the UI renders
 *   percentages, never raw plugin types.
 */
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

/** What the UI needs to know about an available update. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
}

export interface DownloadProgress {
  downloaded: number;
  total?: number;
}

/**
 * True only in a real Tauri webview with the internals reachable. The
 * updater card in About renders only when this says yes (#40 lesson: a
 * permission that was never granted is a silent denial — so the UI asks
 * before it offers).
 */
export function updateSupported(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The pending update handle between check and install (module-private). */
let pending: Update | null = null;

/**
 * Check the release channel once. Resolves null when there is no update OR
 * no updater (outside Tauri) — the caller shows "up to date" only for a real
 * check that came back empty, and hides the card entirely when unsupported.
 */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!updateSupported()) return null;
  const update = await check();
  if (!update) {
    pending = null;
    return null;
  }
  pending = update;
  return { version: update.version, notes: update.body ?? null };
}

/**
 * Download and stage the update the user consented to. Progress rides the
 * callback; resolves when the new version is ready to apply. Requires a
 * prior successful check — refusing is the honest answer, not a re-check.
 */
export async function downloadAndInstall(
  onProgress?: (p: DownloadProgress) => void,
): Promise<void> {
  if (!pending) throw new Error("No update has been checked yet.");
  // The plugin emits per-chunk deltas; this wrapper normalizes them to
  // CUMULATIVE progress so every surface computes a plain percentage.
  let received = 0;
  let total: number | undefined = undefined;
  await pending.downloadAndInstall((event) => {
    if (!onProgress) return;
    switch (event.event) {
      case "Started":
        received = 0;
        total = event.data.contentLength ?? undefined;
        onProgress({ downloaded: 0, total });
        break;
      case "Progress":
        received += event.data.chunkLength;
        onProgress({ downloaded: received, total });
        break;
      case "Finished":
        onProgress({ downloaded: received, total });
        break;
    }
  });
}

/**
 * Apply the staged update by restarting the app. Only meaningful after
 * downloadAndInstall resolved; the restart is the install.
 */
export async function restartToUpdate(): Promise<void> {
  await relaunch();
}
