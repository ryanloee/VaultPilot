// ── Settings cache — standalone module ──────────────────────
// Lives in its own file to break the circular import between client.ts (which
// imports useAppStore from store.ts) and store.ts (which needs to invalidate
// the cache after rehydration). Both modules import this tiny file safely.

type ApiFormat = 'openai' | 'anthropic';

let _settingsCache: {
  apiBase: string;
  apiKey: string;
  model: string;
  apiFormat: ApiFormat;
} | null = null;

/** Reset the cached settings so the next getSettings() re-reads from storage. */
export function invalidateSettingsCache(): void {
  _settingsCache = null;
}

/** Read the current cache value (may be null). */
export function getSettingsCache(): typeof _settingsCache {
  return _settingsCache;
}

/** Write a new value into the cache. */
export function setSettingsCache(
  value: typeof _settingsCache,
): void {
  _settingsCache = value;
}
