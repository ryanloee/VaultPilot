import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "@/lib/tauri";
import { isTauri } from "@/lib/mock";

export type PendingUpdate = Update;

/**
 * Check the configured updater endpoint on desktop Tauri builds.
 * Returns null when running in a browser, on mobile, or when already current.
 */
export async function checkForUpdates(options?: {
  respectPreference?: boolean;
}): Promise<PendingUpdate | null> {
  if (!isTauri() || !(await api.isDesktop())) return null;

  if (options?.respectPreference) {
    const settings = await api.getSettings();
    if (!settings.autoCheckUpdates) return null;
  }

  return check();
}

/** Download a verified update and optionally restart the application. */
export async function installUpdate(update: PendingUpdate): Promise<void> {
  await update.downloadAndInstall();
  const ok = window.confirm("新版本已下载完成，是否立即重启以应用更新？");
  if (ok) await relaunch();
}