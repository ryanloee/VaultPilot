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

export interface SyncResult {
  imported: number;
  updated: number;
  skipped: number;
  errors: number;
  duration_ms: number;
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
    const res = await fetch(`${url}/health`, { signal: AbortSignal.timeout(5000) });
    return res.ok;
  } catch (e) {
    console.warn('[Sync] pingBackend failed:', e);
    return false;
  }
}

/** Full sync: pull notes from backend → local SQLite. */
export async function syncNotesFromServer(): Promise<SyncResult> {
  const start = Date.now();
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
    let listRes: Response | null = null;
    let lastListError: Error | null = null;
    for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
      if (attempt > 0) {
        const delay = RETRY_BASE_MS * Math.pow(2, attempt - 1);
        await new Promise(r => setTimeout(r, delay));
      }
      try {
        listRes = await fetch(`${url}/api/notes?limit=${PAGE_SIZE}&offset=${offset}`, {
          headers,
          signal: AbortSignal.timeout(30000),
        });
        if (listRes.ok) break;
        if (listRes.status >= 500) continue;
        break;
      } catch (fetchErr: unknown) {
        lastListError = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
        if (lastListError.name === 'AbortError') break;
        // network error → retry
      }
    }
    if (!listRes || !listRes.ok) {
      const status = listRes?.status ?? 'network';
      throw new Error(`获取笔记列表失败: ${status}`);
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

  // Get local notes for comparison
  // 只加载 id 和 updated_at，避免全量 content 导致 OOM (#1668)
  const localTimestamps = await getNoteTimestamps();
  const localMap = new Map(localTimestamps.map(n => [n.id, n]));

  for (const meta of allServerNotes) {
    try {
      const serverUpdated = meta.updatedAt ?? meta.updated_at ?? '';
      const serverTs = serverUpdated ? new Date(serverUpdated).getTime() : 0;
      const localNote = localMap.get(meta.id);

      // Skip if local is same or newer
      // localNote.updated_at is in seconds (SQLite strftime('%s')), serverTs is in ms
      if (localNote && localNote.updated_at * 1000 >= serverTs) {
        skipped++;
        continue;
      }

      // Fetch full note (with retry on transient failures, matching client.ts pattern)
      let noteRes: Response | null = null;
      let lastFetchError: Error | null = null;
      for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
        if (attempt > 0) {
          const delay = RETRY_BASE_MS * Math.pow(2, attempt - 1);
          await new Promise(r => setTimeout(r, delay));
        }
        try {
          noteRes = await fetch(`${url}/api/notes/${encodeURIComponent(meta.id)}`, {
            headers,
            signal: AbortSignal.timeout(10000),
          });
          if (noteRes.ok) break; // success
          // Retry on 5xx (transient server errors)
          if (noteRes.status >= 500) continue;
          // 4xx — non-retryable, break immediately
          break;
        } catch (fetchErr: unknown) {
          lastFetchError = fetchErr instanceof Error ? fetchErr : new Error(String(fetchErr));
          if (lastFetchError.name === 'AbortError') break; // don't retry timeouts that user aborted
          // network error → retry
        }
      }
      if (!noteRes || !noteRes.ok) {
        const status = noteRes?.status ?? 'network';
        console.warn(`[Sync] Failed to fetch note ${meta.id} after ${MAX_RETRIES + 1} attempts: HTTP ${status}`);
        errors++;
        continue;
      }

      const noteData = await noteRes.json() as {
        meta: { id: string; title: string; tags?: string[] };
        body: string;
      };

      const title = noteData.meta.title ?? meta.title ?? 'Untitled';
      const content = noteData.body ?? '';

      if (localNote) {
        await updateNote(meta.id, title, content);
        updated++;
      } else {
        await createNote(title, content, meta.id);
        imported++;
      }
    } catch (e) {
      console.warn(`[Sync] Failed: ${meta.id}`, e);
      errors++;
    }
  }

  await AsyncStorage.setItem(LAST_SYNC_KEY, new Date().toISOString());
  return { imported, updated, skipped, errors, duration_ms: Date.now() - start };
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
export async function autoSyncOnStartup(): Promise<AutoSyncResult> {
  try {
    const { url } = await getServerConfig();
    if (!url) return { status: 'skipped', reason: 'no_config' };

    const reachable = await pingBackend();
    if (!reachable) return { status: 'skipped', reason: 'unreachable' };

    console.warn('[Sync] Backend reachable, starting auto-sync...');
    const result = await syncNotesFromServer();
    console.warn('[Sync] Auto-sync complete:', result);
    return { status: 'done', result };
  } catch (e) {
    console.warn('[Sync] Auto-sync failed:', e);
    return { status: 'error', error: String(e) };
  }
}
