/**
 * Vault sync — pull notes from VaultPilot backend server into local SQLite.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { createNote, updateNote, getNoteTimestamps, getNoteTags, addTag, removeTag } from '../db';
import { isRetryable } from '../api/clientUtils';

const SERVER_URL_KEY = 'cfg_backend_url';
const SERVER_TOKEN_KEY = 'cfg_backend_token';
const LAST_SYNC_KEY = 'cfg_last_sync_at';

const MAX_RETRIES = 2;
const RETRY_BASE_MS = 1000;
const RETRY_AFTER_MAX_MS = 60_000; // Retry-After 上限，避免服务端返回过大值导致长时间卡死 (#2132)
const SYNC_OVERALL_TIMEOUT_MS = 5 * 60 * 1000; // 5 minutes
const DETAIL_CONCURRENCY = 5;

export interface SyncResult {
  imported: number;
  updated: number;
  skipped: number;
  errors: number;
  duration_ms: number;
}

export interface SyncProgress {
  phase: 'listing' | 'details';
  total: number;
  completed: number;
  imported: number;
  updated: number;
  skipped: number;
  errors: number;
}

export type SyncProgressCallback = (progress: SyncProgress) => void;

/**
 * Derive the mobile `folder` value from a server note `path` (#2893).
 * The server path encodes the vault-relative location, e.g. "work/meeting.md".
 * The folder is the directory portion ("work"); a note at the vault root
 * ("meeting.md") has an empty folder. Handles both '/' and '\' separators.
 */
export function deriveFolderFromPath(path: string | undefined | null): string {
  if (!path) return '';
  const normalized = path.replace(/\\/g, '/');
  const idx = normalized.lastIndexOf('/');
  return idx >= 0 ? normalized.substring(0, idx) : '';
}

/**
 * Parse a server RFC3339 `updated_at` string into a unix-seconds integer
 * for the local SQLite `updated_at` column (#2893). Returns `undefined`
 * when the value is missing or unparseable, letting the caller fall back
 * to "now".
 */
export function parseServerTimestamp(updatedAt: string | undefined | null): number | undefined {
  if (!updatedAt) return undefined;
  const parsed = Date.parse(updatedAt);
  if (Number.isNaN(parsed)) return undefined;
  return Math.floor(parsed / 1000);
}

export async function getServerConfig(): Promise<{ url: string; token: string }> {
  const [url, token] = await Promise.all([
    AsyncStorage.getItem(SERVER_URL_KEY),
    AsyncStorage.getItem(SERVER_TOKEN_KEY),
  ]);
  return { url: url ?? '', token: token ?? '' };
}

export async function setServerConfig(url: string, token: string): Promise<void> {
  await AsyncStorage.setItem(SERVER_URL_KEY, url.replace(/\/+$/, ''));
  if (token) {
    await AsyncStorage.setItem(SERVER_TOKEN_KEY, token);
  } else {
    await AsyncStorage.removeItem(SERVER_TOKEN_KEY);
  }
}

export async function pingBackend(): Promise<boolean> {
  const { url } = await getServerConfig();
  if (!url) return false;
  try {
    const timeoutController = new AbortController();
    const timer = setTimeout(() => timeoutController.abort(), 5000);
    try {
      const res = await fetch(`${url}/health`, { signal: timeoutController.signal });
      return res.ok;
    } finally {
      clearTimeout(timer);
    }
  } catch (e) {
    console.warn('[Sync] pingBackend failed:', e);
    return false;
  }
}

/** Full sync: pull notes from backend → local SQLite. */
export async function syncNotesFromServer(
  onProgress?: SyncProgressCallback,
): Promise<SyncResult> {
  const start = Date.now();

  // Overall timeout via AbortController (#2011)
  const overallController = new AbortController();
  const overallTimer = setTimeout(
    () => overallController.abort('sync-timeout'),
    SYNC_OVERALL_TIMEOUT_MS,
  );

  try {
    return await doSync(overallController.signal, onProgress, start);
  } finally {
    clearTimeout(overallTimer);
  }
}

/** Internal sync logic so the outer wrapper can clean up the timer. */
async function doSync(
  signal: AbortSignal,
  onProgress: SyncProgressCallback | undefined,
  start: number,
): Promise<SyncResult> {
  const { url, token } = await getServerConfig();
  if (!url) throw new Error('未配置后端服务器地址');

  const headers: Record<string, string> = { Accept: 'application/json' };
  if (token) headers['Authorization'] = `Bearer ${token}`;

  // Paginate through all notes from server (#1398)
  const PAGE_SIZE = 200;
  const MAX_NOTES = 10000; // safety limit
  const allServerNotes: Array<{
    id: string; title: string; tags?: string[]; updatedAt?: string; updated_at?: string;
  }> = [];

  for (let offset = 0; offset < MAX_NOTES; offset += PAGE_SIZE) {
    if (signal.aborted) throw new Error('同步超时');

    let listRes: Response | undefined;
    let lastNetworkErr: Error | undefined;
    let retryAfterMs: number | null = null;
    for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
      if (signal.aborted) throw new Error('同步超时');
      if (attempt > 0) {
        // 优先尊重服务端 Retry-After 头，否则走指数退避 (#2132)
        const delay = retryAfterMs ?? RETRY_BASE_MS * Math.pow(2, attempt - 1);
        retryAfterMs = null;
        await raceDelayOrAbort(signal, delay);
      }
      // 声明在 try 外部，确保 cleanup 在 catch/所有路径都可调用 (#2122)
      const listTimeoutController = new AbortController();
      const listTimeoutTimer = setTimeout(() => listTimeoutController.abort(), 30000);
      try {
        const { signal: fetchSignal, cleanup } = combineSignals(signal, listTimeoutController.signal);
        try {
          listRes = await fetch(`${url}/api/notes?limit=${PAGE_SIZE}&offset=${offset}`, {
            headers,
            signal: fetchSignal,
          });
          if (!listRes) { lastNetworkErr = lastNetworkErr ?? new Error('fetch returned null'); continue; }
          if (isRetryable(listRes.status)) {
            // 429/502/503/504 等可重试状态：尊重 Retry-After 头后重试 (#2132)
            // 必须先取消 response body，否则底层 HTTP 连接泄漏可能耗尽连接池 (#3111)
            await listRes.body?.cancel().catch(() => {});
            retryAfterMs = parseRetryAfter(listRes);
            lastNetworkErr = new Error(`获取笔记列表失败: ${listRes.status}`);
            if (attempt >= MAX_RETRIES) break;
            continue;
          }
          break; // got a response (even if not ok), stop retrying
        } catch (fetchErr: unknown) {
          if (signal.aborted) throw new Error('同步超时');
          lastNetworkErr = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
          if (attempt >= MAX_RETRIES) throw lastNetworkErr;
          // network error → retry
        } finally {
          cleanup();
        }
      } finally {
        clearTimeout(listTimeoutTimer);
      }
    }
    if (!listRes) {
      throw lastNetworkErr ?? new Error('获取笔记列表失败: network');
    }
    if (!listRes.ok) {
      const errBody = await listRes.text().catch(() => '');
      throw new Error(`获取笔记列表失败: ${listRes.status}${errBody ? ` — ${errBody.slice(0, 200)}` : ''}`);
    }

    const { notes } = await listRes.json() as {
      notes: typeof allServerNotes;
      total: number;
    };

    allServerNotes.push(...notes);
    if (notes.length < PAGE_SIZE) break; // last page
  }

  let imported = 0;
  let updated = 0;
  let skipped = 0;
  let errors = 0;

  const emitProgress = (phase: 'listing' | 'details') => {
    if (onProgress) {
      onProgress({ phase, total: allServerNotes.length, completed: imported + updated + skipped + errors, imported, updated, skipped, errors });
    }
  };

  // Get local notes for comparison
  // 只加载 id 和 updated_at，避免全量 content 导致 OOM (#1668)
  const localTimestamps = await getNoteTimestamps();
  const localMap = new Map(localTimestamps.map(n => [n.id, n]));

  // Classify notes: which need detail fetch vs skip
  const notesToFetch: typeof allServerNotes = [];
  for (const meta of allServerNotes) {
    const serverUpdated = meta.updatedAt ?? meta.updated_at ?? '';
    const serverTs = serverUpdated ? new Date(serverUpdated).getTime() : Infinity;
    const localNote = localMap.get(meta.id);
    if (localNote && localNote.updated_at * 1000 >= serverTs) {
      skipped++;
    } else {
      notesToFetch.push(meta);
    }
  }

  emitProgress('details');

  // Concurrent detail fetches with concurrency limit (#2011)
  await runWithConcurrency(notesToFetch, DETAIL_CONCURRENCY, async (meta) => {
    if (signal.aborted) throw new Error('同步超时');

    try {
      // Fetch full note (with retry on transient failures)
      let noteRes: Response | null = null;
      let lastFetchError: Error | null = null;
      let noteRetryAfterMs: number | null = null;
      const noteTimeoutMs = 10000;
      for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
        if (signal.aborted) throw new Error('同步超时');
        if (attempt > 0) {
          // 优先尊重服务端 Retry-After 头，否则走指数退避 (#2132)
          const delay = noteRetryAfterMs ?? RETRY_BASE_MS * Math.pow(2, attempt - 1);
          noteRetryAfterMs = null;
          await raceDelayOrAbort(signal, delay);
        }
        const noteController = new AbortController();
        let abortedDueToTimeout = false;
        const timer = setTimeout(() => { abortedDueToTimeout = true; noteController.abort('timeout'); }, noteTimeoutMs);
        // 声明在 try 外部，确保 cleanup 在 catch/所有路径都可调用 (#2122)
        const { signal: noteSignal, cleanup: noteCleanup } = combineSignals(signal, noteController.signal);
        try {
          noteRes = await fetch(`${url}/api/notes/${encodeURIComponent(meta.id)}`, {
            headers,
            signal: noteSignal,
          });
          clearTimeout(timer);
          if (noteRes.ok) break; // success
          // 可重试状态（429/502/503/504）：尊重 Retry-After 头后重试 (#2132)
          if (isRetryable(noteRes.status)) {
            // 必须先取消 response body，否则底层 HTTP 连接泄漏可能耗尽连接池 (#3111)
            await noteRes.body?.cancel().catch(() => {});
            noteRetryAfterMs = parseRetryAfter(noteRes);
            continue;
          }
          // 4xx 等不可重试状态 — 必须 cancel body 后放弃，否则底层 HTTP 连接
          // 泄漏会逐步耗尽 fetch 连接池 (#3119). 与上方 isRetryable 分支的
          // cancel 处理对称（#3111）.
          await noteRes.body?.cancel().catch(() => {});
          break;
        } catch (fetchErr: unknown) {
          clearTimeout(timer);
          if (signal.aborted) return;
          lastFetchError = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
          // Only break on user-initiated abort; timeout aborts should be retried
          if (lastFetchError.name === 'AbortError' && !abortedDueToTimeout) break;
          // network error or timeout → retry
        } finally {
          noteCleanup();
        }
      }
      if (!noteRes || !noteRes.ok) {
        const status = noteRes?.status ?? 'network';
        console.warn(`[Sync] Failed to fetch note ${meta.id} after ${MAX_RETRIES + 1} attempts: HTTP ${status}`);
        errors++;
        emitProgress('details');
        return;
      }

      const noteData = await noteRes.json() as {
        meta: { id: string; title: string; tags?: string[]; is_template?: number; path?: string; updated_at?: string };
        body: string;
      };

      const title = noteData.meta.title ?? meta.title ?? 'Untitled';
      const content = noteData.body ?? '';

      // Propagate server folder + real edit time into the local note (#2893).
      const folder = deriveFolderFromPath(noteData.meta.path);
      const updatedAt = parseServerTimestamp(noteData.meta.updated_at);

      const localNote = localMap.get(meta.id);
      if (localNote) {
        await updateNote(meta.id, title, content, {
          skipQueue: true,
          is_template: noteData.meta.is_template ?? 0,
          folder,
          updated_at: updatedAt,
        });
        updated++;
      } else {
        await createNote(title, content, meta.id, {
          skipQueue: true,
          is_template: noteData.meta.is_template ?? 0,
          folder,
          updated_at: updatedAt,
        });
        imported++;
      }
      // Sync tags from server to local database (#2477)
      const serverTags = noteData.meta.tags ?? [];
      const localTags = await getNoteTags(meta.id);
      for (const tag of serverTags) {
        if (!localTags.includes(tag)) {
          await addTag(meta.id, tag, { skipQueue: true });
        }
      }
      for (const tag of localTags) {
        if (!serverTags.includes(tag)) {
          await removeTag(meta.id, tag, { skipQueue: true });
        }
      }
      emitProgress('details');
    } catch (e) {
      if (signal.aborted) throw new Error('同步超时');
      console.warn(`[Sync] Failed: ${meta.id}`, e);
      errors++;
      emitProgress('details');
    }
  }, signal);

  // Only persist the sync timestamp on a clean (non-aborted) sync (#2369)
  if (!signal.aborted) {
    await AsyncStorage.setItem(LAST_SYNC_KEY, new Date().toISOString());
  }
  return { imported, updated, skipped, errors, duration_ms: Date.now() - start };
}

/** Run async tasks with a concurrency limit. */
async function runWithConcurrency<T>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<void>,
  signal?: AbortSignal,
): Promise<void> {
  let index = 0;
  let aborted = false;
  const innerController = new AbortController();
  // Propagate outer signal to inner controller
  const onOuterAbort = () => innerController.abort();
  signal?.addEventListener('abort', onOuterAbort, { once: true });
  const innerSignal = innerController.signal;

  try {
    const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
      while (index < items.length) {
        if (innerSignal.aborted) { aborted = true; return; }
        const i = index++;
        try { await fn(items[i]); } catch (e) {
          if (innerSignal.aborted) { aborted = true; return; }
          // Cancel other workers when one fails (#3534)
          innerController.abort();
          throw e;
        }
      }
    });
    await Promise.all(workers);
    if (aborted) return;  // Return partial results instead of throwing (#2451)
  } finally {
    // Clean up the outer-signal listener to prevent listener leak (#3536)
    signal?.removeEventListener('abort', onOuterAbort);
  }
}

/**
 * 解析 Retry-After 响应头（秒数或 HTTP 日期）为毫秒数，用于限流/重试退避。
 * 头缺失或无法解析时返回 null。结果上限 RETRY_AFTER_MAX_MS。(#2132)
 */
export function parseRetryAfter(res: Response): number | null {
  const header = res.headers?.get?.('retry-after');
  if (header == null || header === '') return null;
  // 数字形式：秒数
  const seconds = Number(header);
  if (Number.isFinite(seconds) && seconds >= 0) {
    return Math.min(seconds * 1000, RETRY_AFTER_MAX_MS);
  }
  // HTTP-date 形式
  const date = Date.parse(header);
  if (!Number.isNaN(date)) {
    return Math.min(Math.max(0, date - Date.now()), RETRY_AFTER_MAX_MS);
  }
  return null;
}

/**
 * Wait for `delay` ms, but resolve immediately if `signal` aborts (#2552).
 * Ensures retry backoff / Retry-After delays don't block sync exit past the
 * overall timeout or user cancellation. Exported for unit testing.
 */
export function raceDelayOrAbort(signal: AbortSignal, delay: number): Promise<void> {
  if (signal.aborted) return Promise.resolve();
  return new Promise<void>(resolve => {
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', onAbort); };
    const done = () => { cleanup(); resolve(); };
    const onAbort = done;
    const timer = setTimeout(done, delay);
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

/** Combine multiple AbortSignals into one. Returns a merged signal that aborts when any source aborts. */
function combineSignals(...signals: AbortSignal[]): { signal: AbortSignal; cleanup: () => void } {
  const combined = new AbortController();
  const handlers: Array<[AbortSignal, () => void]> = [];
  for (const s of signals) {
    if (s.aborted) {
      for (const [sig, h] of handlers) {
        sig.removeEventListener('abort', h);
      }
      combined.abort(s.reason);
      return { signal: combined.signal, cleanup: () => {} };
    }
    const handler = () => combined.abort(s.reason);
    s.addEventListener('abort', handler, { once: true });
    handlers.push([s, handler]);
  }
  return {
    signal: combined.signal,
    cleanup: () => {
      for (const [s, h] of handlers) {
        s.removeEventListener('abort', h);
      }
    },
  };
}

export async function getLastSyncTime(): Promise<string | null> {
  return AsyncStorage.getItem(LAST_SYNC_KEY);
}

export type AutoSyncResult =
  | { status: 'skipped'; reason: 'no_config' | 'unreachable' }
  | { status: 'done'; result: SyncResult }
  | { status: 'error'; error: string };

/**
 * Auto-sync on startup: if backend is configured and reachable, sync notes.
 * Returns a discriminated union so callers can distinguish "not configured"
 * from "backend unreachable" from "sync error".
 * Non-blocking — errors are caught and logged, never thrown.
 */
export async function autoSyncOnStartup(onProgress?: SyncProgressCallback): Promise<AutoSyncResult> {
  try {
    const { url } = await getServerConfig();
    if (!url) return { status: 'skipped', reason: 'no_config' };

    const reachable = await pingBackend();
    if (!reachable) return { status: 'skipped', reason: 'unreachable' };

    console.warn('[Sync] Backend reachable, starting auto-sync...');
    const result = await syncNotesFromServer(onProgress);
    console.warn('[Sync] Auto-sync complete:', result);
    return { status: 'done', result };
  } catch (e) {
    console.warn('[Sync] Auto-sync failed:', e);
    return { status: 'error', error: String(e) };
  }
}
