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
  incrementPendingSyncRetry,
  getPendingSyncRetryCount,
} from '../db';
import { getServerConfig } from '../services/sync';

/** Maximum retry attempts for server errors (5xx) before giving up. */
const MAX_RETRY_ATTEMPTS = 5;

/** Hook: returns pending sync count and auto-flushes when online. */
export function usePendingSync(): { pendingCount: number; refresh: () => Promise<void> } {
  const [pendingCount, setPendingCount] = useState(0);
  const { isOnline } = useNetworkState();
  const prevOnline = useRef(isOnline);
  const isFlushingRef = useRef(false);

  const refresh = useCallback(async () => {
    const count = await getPendingSyncCount();
    setPendingCount(count);
  }, []);

  // Keep a ref to the latest refresh callback so the online-transition
  // effect never captures a stale closure (fixes #2010).
  const refreshRef = useRef(refresh);
  useEffect(() => { refreshRef.current = refresh; }, [refresh]);

  // Initial load
  useEffect(() => {
    refresh().catch(e => console.warn('[OfflineSync] refresh failed:', e));
  }, [refresh]);

  // Auto-flush when transitioning from offline → online
  useEffect(() => {
    let cancelled = false;
    if (isOnline && !prevOnline.current && !isFlushingRef.current) {
      isFlushingRef.current = true;
      (async () => {
        try {
          await flushPendingSyncs();
          if (!cancelled) refreshRef.current();
        } catch (e) {
          console.warn('[OfflineSync] flush failed:', e);
        } finally {
          isFlushingRef.current = false;
        }
      })();
    }
    prevOnline.current = isOnline;
    return () => { cancelled = true; };
  }, [isOnline]);

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

      const timeoutController = new AbortController();
      const timer = setTimeout(() => timeoutController.abort(), 10000);
      let res: Response;
      try {
        res = await fetch(`${url}/api/notes/${encodeURIComponent(entry.note_id)}`, {
          method: 'PUT',
          headers,
          body: JSON.stringify({ title: note.title, content: note.content }),
          signal: timeoutController.signal,
        });
      } finally {
        clearTimeout(timer);
      }

      if (res.ok) {
        await clearPendingSync(entry.note_id);
        synced++;
      } else if (res.status === 429) {
        // Rate limited: stop the entire flush to avoid hammering an already-limited server.
        // Remaining entries will be retried on the next flush.
        console.warn(`[OfflineSync] rate limited for note ${entry.note_id}: ${res.status}, pausing flush`);
        failed++;
        break;
      } else if (res.status >= 400 && res.status < 500) {
        // Client error (4xx): clear entry, won't succeed on retry
        console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: client error ${res.status}`);
        await clearPendingSync(entry.note_id);
        failed++;
      } else {
        // Server error (5xx): increment retry count
        await incrementPendingSyncRetry(entry.note_id);
        const retryCount = await getPendingSyncRetryCount(entry.note_id);
        if (retryCount >= MAX_RETRY_ATTEMPTS) {
          console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: max retries exceeded`);
          await clearPendingSync(entry.note_id);
        }
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
