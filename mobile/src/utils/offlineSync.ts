/**
 * Offline sync queue — flushes pending edits when connectivity returns (#1220).
 *
 * When the device is offline, note edits are saved locally and queued.
 * When back online, this module attempts to sync them to the backend server.
 */

import { useEffect, useRef, useCallback, useState } from 'react';
import { useNetworkState } from './networkState';
import {
  getPendingSyncCount,
  getPendingSyncs,
  clearPendingSync,
  getNote,
  queuePendingSync,
} from '../db';
import { getServerConfig } from '../services/sync';

/** Hook: returns pending sync count and auto-flushes when online. */
export function usePendingSync(): { pendingCount: number; refresh: () => Promise<void> } {
  const [pendingCount, setPendingCount] = useState(0);
  const { isOnline } = useNetworkState();
  const prevOnline = useRef(isOnline);

  const refresh = useCallback(async () => {
    const count = await getPendingSyncCount();
    setPendingCount(count);
  }, []);

  // Initial load
  useEffect(() => {
    refresh().catch(e => console.warn('[OfflineSync] refresh failed:', e));
  }, [refresh]);

  // Auto-flush when transitioning from offline → online
  useEffect(() => {
    let cancelled = false;
    if (isOnline && !prevOnline.current) {
      flushPendingSyncs()
        .then(() => { if (!cancelled) refresh(); })
        .catch(e => console.warn('[OfflineSync] flush failed:', e));
    }
    prevOnline.current = isOnline;
    return () => { cancelled = true; };
  }, [isOnline, refresh]);

  return { pendingCount, refresh };
}

/** Attempt to sync all pending notes to the backend server. */
export async function flushPendingSyncs(): Promise<{ synced: number; failed: number }> {
  const { url, token } = await getServerConfig();
  if (!url) return { synced: 0, failed: 0 };

  const pending = await getPendingSyncs();
  let synced = 0;
  let failed = 0;

  for (const entry of pending) {
    try {
      const note = await getNote(entry.note_id);
      if (!note) {
        // Note was deleted locally, just clear the pending entry
        await clearPendingSync(entry.note_id);
        synced++;
        continue;
      }

      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
        'Accept': 'application/json',
      };
      if (token) headers['Authorization'] = `Bearer ${token}`;

      const res = await fetch(`${url}/api/notes/${encodeURIComponent(entry.note_id)}`, {
        method: 'PUT',
        headers,
        body: JSON.stringify({ title: note.title, content: note.content }),
        signal: AbortSignal.timeout(10000),
      });

      if (res.ok) {
        await clearPendingSync(entry.note_id);
        synced++;
      } else {
        failed++;
      }
    } catch (e) {
      console.warn('[OfflineSync] flush failed for note:', entry.note_id, e);
      failed++;
      // Stop flushing if network error — will retry next time
      break;
    }
  }

  return { synced, failed };
}
