/**
 * Vault sync — pull notes from VaultPilot backend server into local SQLite.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { createNote, updateNote, getNoteTimestamps } from '../db';

const SERVER_URL_KEY = 'cfg_backend_url';
const SERVER_TOKEN_KEY = 'cfg_backend_token';
const LAST_SYNC_KEY = 'cfg_last_sync_at';

const MAX_RETRIES = 2;
const RETRY_BASE_MS = 1000;
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
    const res = await fetch(`${url}/health`, { signal: AbortSignal.timeout(5000) });
    return res.ok;
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
    for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
      if (signal.aborted) throw new Error('同步超时');
      if (attempt > 0) {
        const delay = RETRY_BASE_MS * Math.pow(2, attempt - 1);
        await new Promise(r => setTimeout(r, delay));
      }
      try {
        const { signal: fetchSignal, cleanup } = combineSignals(signal, AbortSignal.timeout(30000));
        listRes = await fetch(`${url}/api/notes?limit=${PAGE_SIZE}&offset=${offset}`, {
          headers,
          signal: fetchSignal,
        });
        cleanup();
        if (!listRes) { lastNetworkErr = lastNetworkErr ?? new Error('fetch returned null'); continue; }
        if (listRes.status >= 500) {
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
    if (signal.aborted) return;

    try {
      // Fetch full note (with retry on transient failures)
      let noteRes: Response | null = null;
      let lastFetchError: Error | null = null;
      const noteTimeoutMs = 10000;
      for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
        if (signal.aborted) return;
        if (attempt > 0) {
          const delay = RETRY_BASE_MS * Math.pow(2, attempt - 1);
          await new Promise(r => setTimeout(r, delay));
        }
        const noteController = new AbortController();
        const timer = setTimeout(() => noteController.abort('timeout'), noteTimeoutMs);
        try {
          const { signal: noteSignal, cleanup: noteCleanup } = combineSignals(signal, noteController.signal);
          noteRes = await fetch(`${url}/api/notes/${encodeURIComponent(meta.id)}`, {
            headers,
            signal: noteSignal,
          });
          noteCleanup();
          clearTimeout(timer);
          if (noteRes.ok) break; // success
          // Retry on 5xx (transient server errors)
          if (noteRes.status >= 500) continue;
          // 4xx — non-retryable, break immediately
          break;
        } catch (fetchErr: unknown) {
          clearTimeout(timer);
          if (signal.aborted) return;
          lastFetchError = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
          // Only break on user-initiated abort; timeout aborts should be retried
          if (lastFetchError.name === 'AbortError' && noteController.signal.reason !== 'timeout') break;
          // network error or timeout → retry
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
        meta: { id: string; title: string; tags?: string[] };
        body: string;
      };

      const title = noteData.meta.title ?? meta.title ?? 'Untitled';
      const content = noteData.body ?? '';

      const localNote = localMap.get(meta.id);
      if (localNote) {
        await updateNote(meta.id, title, content);
        updated++;
      } else {
        await createNote(title, content, meta.id);
        imported++;
      }
      emitProgress('details');
    } catch (e) {
      if (signal.aborted) return;
      console.warn(`[Sync] Failed: ${meta.id}`, e);
      errors++;
      emitProgress('details');
    }
  });

  await AsyncStorage.setItem(LAST_SYNC_KEY, new Date().toISOString());
  return { imported, updated, skipped, errors, duration_ms: Date.now() - start };
}

/** Run async tasks with a concurrency limit. */
async function runWithConcurrency<T>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<void>,
): Promise<void> {
  let index = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (index < items.length) {
      const i = index++;
      await fn(items[i]);
    }
  });
  await Promise.all(workers);
}

/** Combine multiple AbortSignals into one. Returns a merged signal that aborts when any source aborts. */
function combineSignals(...signals: AbortSignal[]): { signal: AbortSignal; cleanup: () => void } {
  const combined = new AbortController();
  const handlers: Array<[AbortSignal, () => void]> = [];
  for (const s of signals) {
    if (s.aborted) {
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
