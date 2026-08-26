import { isTauri } from "@/lib/mock";

/**
 * Native OS notifications (Windows toast / Android system notification) via
 * tauri-plugin-notification.
 *
 * Android 13+ requires the POST_NOTIFICATIONS runtime permission — request it
 * once at app start (`ensureNotificationPermission`). Windows toasts are
 * granted by default, so the request is a harmless no-op there.
 *
 * Rust-side notifications (pairing events) go through the plugin directly;
 * this module is the frontend entry point for JS-triggered notifications.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    const { isPermissionGranted, requestPermission } = await import(
      "@tauri-apps/plugin-notification"
    );
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === "granted";
  } catch {
    return false;
  }
}

/** Fire a native notification; silently ignored in browser/mock mode or when
 * the permission was denied. */
export async function nativeNotify(title: string, body?: string): Promise<void> {
  if (!isTauri()) return;
  try {
    const { isPermissionGranted, sendNotification } = await import(
      "@tauri-apps/plugin-notification"
    );
    if (!(await isPermissionGranted())) return;
    sendNotification({ title, body });
  } catch {
    /* notification failures must never break the app */
  }
}
