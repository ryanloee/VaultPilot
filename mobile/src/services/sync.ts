/**
 * Vault sync — pull notes from VaultPilot backend server into local SQLite.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { createNote, updateNote, getNote, getNotes, type DbNote } from '../db';

const SERVER_URL_KEY = 'cfg_backend_url';
const SERVER_TOKEN_KEY = 'cfg_backend_token';
const LAST_SYNC_KEY = 'cfg_last_sync_at';

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
    const listRes = await fetch(`${url}/api/notes?limit=${PAGE_SIZE}&offset=${offset}`, {
      headers,
      signal: AbortSignal.timeout(30000),
    });
    if (!listRes.ok) throw new Error(`获取笔记列表失败: ${listRes.status}`);

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
  const localNotes = await getNotes();
  const localMap = new Map(localNotes.map(n => [n.id, n]));

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

      // Fetch full note
      const noteRes = await fetch(`${url}/api/notes/${encodeURIComponent(meta.id)}`, {
        headers,
        signal: AbortSignal.timeout(10000),
      });
      if (!noteRes.ok) { errors++; continue; }

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

/**
 * Auto-sync on startup: if backend is configured and reachable, sync notes.
 * Returns the sync result if sync was attempted, null if skipped.
 * Non-blocking — errors are caught and logged, never thrown.
 */
export async function autoSyncOnStartup(): Promise<SyncResult | null> {
  try {
    const { url } = await getServerConfig();
    if (!url) return null;

    const reachable = await pingBackend();
    if (!reachable) return null;

    console.log('[Sync] Backend reachable, starting auto-sync...');
    const result = await syncNotesFromServer();
    console.log('[Sync] Auto-sync complete:', result);
    return result;
  } catch (e) {
    console.warn('[Sync] Auto-sync failed:', e);
    return null;
  }
}
