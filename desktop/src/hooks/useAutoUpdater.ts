import { useEffect } from "react";
import { isTauri } from "@/lib/mock";
import { checkForUpdates, installUpdate } from "@/lib/updater";

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
        // The updater plugin is desktop-only. Keep Android/iOS builds from
        // calling an unsupported API, and honor the persisted user setting.
        const update = await checkForUpdates({ respectPreference: true });
        if (cancelled || !update) return; // unsupported or already latest

        await installUpdate(update);
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
