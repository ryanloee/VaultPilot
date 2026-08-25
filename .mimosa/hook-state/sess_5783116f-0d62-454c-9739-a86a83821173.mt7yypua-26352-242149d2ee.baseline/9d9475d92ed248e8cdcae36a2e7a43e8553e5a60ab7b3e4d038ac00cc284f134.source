import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "@/lib/tauri";
import { isTauri } from "@/lib/mock";
import { useUpdaterStore } from "@/lib/store";

export type PendingUpdate = Update;

/**
 * Check the configured updater endpoint on desktop Tauri builds.
 * Returns null when running in a browser, on mobile, or when already current.
 */
export async function checkForUpdates(options?: {
  respectPreference?: boolean;
}): Promise<PendingUpdate | null> {
  const store = useUpdaterStore.getState();
  if (!isTauri() || !(await api.isDesktop())) return null;

  if (options?.respectPreference) {
    const settings = await api.getSettings();
    if (!settings.autoCheckUpdates) return null;
  }

  store.setPhase("checking");
  try {
    const update = await check();
    if (!update) {
      store.reset();
    } else {
      store.setVersion(update.version);
    }
    return update;
  } catch (e) {
    store.setError(String(e));
    return null;
  }
}

/** Download a verified update with progress tracking, then prompt to restart.
 * Progress is written to the global updater store so any page can render it
 * — switching views mid-download does not lose state. */
export async function installUpdate(update: PendingUpdate): Promise<void> {
  const store = useUpdaterStore.getState();
  store.setPhase("downloading");
  store.setProgress(0, 0);
  store.setVersion(update.version);

  let contentLength = 0;
  let downloaded = 0;
  try {
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          contentLength = event.data.contentLength ?? 0;
          useUpdaterStore.getState().setProgress(0, contentLength);
          break;
        case "Progress":
          downloaded += event.data.chunkLength;
          useUpdaterStore.getState().setProgress(downloaded, contentLength);
          break;
        case "Finished":
          useUpdaterStore.getState().setProgress(contentLength, contentLength);
          useUpdaterStore.getState().setPhase("ready");
          break;
      }
    });

    useUpdaterStore.getState().setPhase("ready");
    const ok = window.confirm("新版本已下载完成，是否立即重启以应用更新？");
    if (ok) {
      await relaunch();
    } else {
      // Keep "ready" state so the user can relaunch from Settings later.
    }
  } catch (e) {
    useUpdaterStore.getState().setError(String(e));
    throw e;
  }
}