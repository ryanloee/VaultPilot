/**
 * Background sync — periodically pull notes from the VaultPilot backend
 * even when the app is not in the foreground (#3158).
 *
 * Uses `expo-background-task` (which hooks into Android WorkManager / iOS
 * BackgroundTasks) to schedule a periodic task whose body simply calls the
 * existing foreground sync logic (`syncNotesFromServer`).
 *
 * Configuration is persisted to AsyncStorage and applied at app launch via
 * `applyBackgroundSyncFromConfig()`. The feature is **off by default** per
 * the issue spec (user must explicitly enable it in Settings → 数据同步).
 *
 * Design notes
 * ------------
 * - The task body must be side-effect-light and resilient: background tasks
 *   run in an isolated JS context with a strict budget. All errors are
 *   swallowed and logged so the OS does not penalise us.
 * - `TaskManager.defineTask` must be called at module evaluation time
 *   (top-level) so the task is registered before the background event fires.
 *   We guard with a feature-detection check so Jest (where the native module
 *   is mocked/absent) does not crash.
 * - A single task id is used so re-registering with a different interval is
 *   an idempotent update.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer, getServerConfig, pingBackend } from './sync';

/** AsyncStorage keys (mirrored by SettingsScreen). */
export const BG_SYNC_ENABLED_KEY = 'cfg_bg_sync_enabled';
export const BG_SYNC_INTERVAL_KEY = 'cfg_bg_sync_interval';

/** Stable identifier used to register the OS-level periodic task. */
export const BACKGROUND_SYNC_TASK_ID = 'vaultpilot-bg-sync';

/** Minimum interval supported by the OS scheduler (~15 min on Android). */
export const MIN_INTERVAL_MINUTES = 15;

export type BackgroundSyncInterval = 15 | 30 | 60;

export interface BackgroundSyncConfig {
  enabled: boolean;
  intervalMinutes: BackgroundSyncInterval;
}

export const DEFAULT_CONFIG: BackgroundSyncConfig = {
  enabled: false,
  intervalMinutes: 30,
};

/**
 * Native modules are lazy-imported inside a try/catch so this file can be
 * imported in non-RN environments (Jest, SSR) without throwing. When the
 * modules are unavailable we degrade gracefully: configuration still works
 * (persisted to AsyncStorage) but no OS task is registered.
 */
let BackgroundTask: typeof import('expo-background-task') | null = null;
let TaskManager: typeof import('expo-task-manager') | null = null;

try {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  BackgroundTask = require('expo-background-task');
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  TaskManager = require('expo-task-manager');
} catch {
  // Native modules not available (e.g. Jest). Feature-detection handles it.
  BackgroundTask = null;
  TaskManager = null;
}

/** True only when both native modules are available at runtime. */
export function isBackgroundSyncAvailable(): boolean {
  return BackgroundTask !== null && TaskManager !== null;
}

/**
 * Register the task body with TaskManager. Idempotent — guarded by
 * `isTaskDefined` so we never double-define on hot-reload.
 */
function registerTaskBody(): void {
  if (!TaskManager) return;
  try {
    if (!TaskManager.isTaskDefined(BACKGROUND_SYNC_TASK_ID)) {
      TaskManager.defineTask(BACKGROUND_SYNC_TASK_ID, backgroundSyncTaskBody);
    }
  } catch (e) {
    /* already defined — only suppress duplicate-registration, warn for real failures */
    console.warn('[BgSync] Failed to register background task:', e);
  }
}

/**
 * The actual work performed during a background fetch window.
 * Exported for unit testing (mocked in regression tests).
 *
 * Returns the numeric `BackgroundTaskResult` value:
 *   Success = 1, Failed = 2.
 * Falls back to raw numbers when the native module is absent (Jest).
 *
 * Result semantics (#3176):
 *   - The task **executed without throwing** → `Success`. "Nothing to sync"
 *     (no backend configured) and "backend temporarily unreachable" are
 *     normal outcomes, not execution failures. Returning `Failed` for them
 *     makes iOS map the result to `UIBackgroundFetchResult.failed`, which
 *     Apple uses to gradually reduce the app's background-fetch budget.
 *   - Only a thrown exception (genuine execution failure) → `Failed`.
 */
export async function backgroundSyncTaskBody(): Promise<number> {
  try {
    const { url } = await getServerConfig();
    if (!url) return resultSuccess();

    // Quick liveness check before the heavier sync to avoid wasting the
    // background budget when the backend is offline.
    const reachable = await pingBackend();
    if (!reachable) return resultSuccess();

    await syncNotesFromServer();
    return resultSuccess();
  } catch (e) {
    console.warn('[BgSync] task failed:', e);
    return resultFailed();
  }
}

/** Map success to the BackgroundTaskResult.Success enum value (1). */
function resultSuccess(): number {
  if (BackgroundTask && BackgroundTask.BackgroundTaskResult) {
    return BackgroundTask.BackgroundTaskResult.Success;
  }
  return 1;
}

/** Map failure to the BackgroundTaskResult.Failed enum value (2). */
function resultFailed(): number {
  if (BackgroundTask && BackgroundTask.BackgroundTaskResult) {
    return BackgroundTask.BackgroundTaskResult.Failed;
  }
  return 2;
}

// Register the task body once at module load (no-op if modules are missing).
registerTaskBody();

/**
 * Read the persisted configuration from AsyncStorage. Returns defaults when
 * nothing is stored yet.
 */
export async function getBackgroundSyncConfig(): Promise<BackgroundSyncConfig> {
  const [enabledRaw, intervalRaw] = await Promise.all([
    AsyncStorage.getItem(BG_SYNC_ENABLED_KEY),
    AsyncStorage.getItem(BG_SYNC_INTERVAL_KEY),
  ]);
  const enabled = enabledRaw === 'true';
  let intervalMinutes: BackgroundSyncInterval = DEFAULT_CONFIG.intervalMinutes;
  if (intervalRaw === '15' || intervalRaw === '30' || intervalRaw === '60') {
    intervalMinutes = Number(intervalRaw) as BackgroundSyncInterval;
  }
  return { enabled, intervalMinutes };
}

/**
 * Persist configuration and (un)register the OS-level task accordingly.
 * Safe to call repeatedly; registration is idempotent.
 */
export async function configureBackgroundSync(
  enabled: boolean,
  intervalMinutes: BackgroundSyncInterval,
): Promise<BackgroundSyncConfig> {
  // Clamp to the supported minimum.
  const clamped = Math.max(MIN_INTERVAL_MINUTES, intervalMinutes) as BackgroundSyncInterval;

  await Promise.all([
    AsyncStorage.setItem(BG_SYNC_ENABLED_KEY, enabled ? 'true' : 'false'),
    AsyncStorage.setItem(BG_SYNC_INTERVAL_KEY, String(clamped)),
  ]);

  if (!isBackgroundSyncAvailable()) {
    // Config persisted; native side will be applied on next app launch
    // that has the native modules (real device).
    return { enabled, intervalMinutes: clamped };
  }

  // #3225: Track OS registration outcome. If registerTaskAsync throws,
  // we must NOT return enabled:true — that would make the UI show sync as
  // ON when no OS-level task is actually registered (silent failure).
  try {
    // Always unregister first to clear any prior interval.
    try {
      await BackgroundTask!.unregisterTaskAsync(BACKGROUND_SYNC_TASK_ID);
    } catch {
      // Task may not be registered yet — ignore.
    }

    if (enabled) {
      await BackgroundTask!.registerTaskAsync(BACKGROUND_SYNC_TASK_ID, {
        minimumInterval: clamped,
      });
    }
  } catch (e) {
    console.warn('[BgSync] configure failed:', e);
    // #3225: Registration failed: persist the corrected state so the UI
    // doesn't display an optimistic enabled value that doesn't match OS
    // reality. Disabled-state failures (e.g., unregister) are harmless.
    if (enabled) {
      await AsyncStorage.setItem(BG_SYNC_ENABLED_KEY, 'false');
      return { enabled: false, intervalMinutes: clamped };
    }
  }

  return { enabled, intervalMinutes: clamped };
}

/**
 * Apply whatever is persisted in AsyncStorage to the OS scheduler.
 * Called once at app launch. Idempotent.
 */
export async function applyBackgroundSyncFromConfig(): Promise<BackgroundSyncConfig> {
  const cfg = await getBackgroundSyncConfig();
  // Re-apply so any config change made while native modules were unavailable
  // (e.g. edited in Jest) is reflected once we run on a real device.
  return configureBackgroundSync(cfg.enabled, cfg.intervalMinutes);
}

/** Human-readable label for an interval value (Chinese UI). */
export function intervalLabel(minutes: BackgroundSyncInterval): string {
  switch (minutes) {
    case 15: return '15 分钟';
    case 30: return '30 分钟';
    case 60: return '1 小时';
    default: return `${minutes} 分钟`;
  }
}
