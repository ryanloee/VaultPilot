import { useEffect } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isTauri } from "@/lib/mock";

/**
 * Auto-update check on startup (desktop Tauri builds only).
 *
 * Queries the updater endpoint (GitHub release latest.json) once on app
 * launch. If a newer version exists, downloads it and prompts the user to
 * relaunch. Silent failures are swallowed — an update check must never
 * block app startup or crash the UI (offline / no new version / no
 * signature are all expected non-events).
 */
export function useAutoUpdater() {
  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    (async () => {
      try {
        const update = await check();
        if (cancelled || !update) return; // already latest

        await update.downloadAndInstall();
        if (cancelled) return;

        // Ask before relaunching so the user can save work first.
        const ok = window.confirm(
          "新版本已下载完成，是否立即重启以应用更新？",
        );
        if (ok) await relaunch();
      } catch (error) {
        // Offline, no release yet, signature mismatch, etc. — ignore.
        console.debug("[updater] check skipped:", error);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);
}
