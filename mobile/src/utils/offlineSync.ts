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
/** Maximum retry attempts per flush cycle for transient fetch errors. */
const FLUSH_RETRIES = 2;
/** Base delay (ms) for exponential backoff between retry attempts. */
const RETRY_BASE_MS = 1000;

/** Hook: returns pending sync count and auto-flushes when online. */
export function usePendingSync(): { pendingCount: number; refresh: () => Promise<void> } {
  const [pendingCount, setPendingCount] = useState(0);
  const { isOnline } = useNetworkState();
  const prevOnline = useRef(false);
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
    return () => { cancelled = true; isFlushingRef.current = false; };
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

      // Handle delete action: send DELETE request to server (#2433)
      if (entry.action === 'delete') {
        const headers: Record<string, string> = {
          'Content-Type': 'application/json',
          'Accept': 'application/json',
        };
        if (token) headers['Authorization'] = `Bearer ${token}`;

        let shouldStopFlush = false;

        for (let attempt = 0; attempt <= FLUSH_RETRIES; attempt++) {
          if (attempt > 0) {
            // Exponential backoff: 1s, 2s
            await new Promise((r) => setTimeout(r, RETRY_BASE_MS * Math.pow(2, attempt - 1)));
          }

          const timeoutController = new AbortController();
          const timer = setTimeout(() => timeoutController.abort(), 10000);
          try {
            const res = await fetch(`${url}/api/notes/${encodeURIComponent(entry.note_id)}`, {
              method: 'DELETE',
              headers,
              signal: timeoutController.signal,
            });

            if (res.ok) {
              await clearPendingSync(entry.note_id);
              synced++;
              break; // success
            }

            if (res.status === 429) {
              console.warn(`[OfflineSync] rate limited on delete for note ${entry.note_id}: ${res.status}, pausing flush`);
              failed++;
              shouldStopFlush = true;
              break; // rate limited, stop entire flush
            }

            if (res.status === 408) {
              // Request Timeout: transient, retry on next flush (#2502)
              console.warn(`[OfflineSync] request timeout for delete of note ${entry.note_id}: ${res.status}, will retry`);
              await incrementPendingSyncRetry(entry.note_id);
              const retryCount = await getPendingSyncRetryCount(entry.note_id);
              if (retryCount >= MAX_RETRY_ATTEMPTS) {
                console.warn(`[OfflineSync] clearing delete entry for note ${entry.note_id}: max retries exceeded (408)`);
                await clearPendingSync(entry.note_id);
              }
              failed++;
              break;
            }

            if (res.status >= 400 && res.status < 500) {
              console.warn(`[OfflineSync] clearing delete entry for note ${entry.note_id}: client error ${res.status}`);
              await clearPendingSync(entry.note_id);
              failed++;
              break; // client error, won't succeed on retry
            }

            // 5xx or unknown server error — retry if attempts remain
            if (attempt >= FLUSH_RETRIES) {
              // Last attempt failed — fall back to cross-cycle retry
              await incrementPendingSyncRetry(entry.note_id);
              const retryCount = await getPendingSyncRetryCount(entry.note_id);
              if (retryCount >= MAX_RETRY_ATTEMPTS) {
                console.warn(`[OfflineSync] clearing delete entry for note ${entry.note_id}: max retries exceeded`);
                await clearPendingSync(entry.note_id);
              }
              failed++;
            }
            // else: transient 5xx, loop continues with exponential backoff
          } catch (fetchErr) {
            if (attempt >= FLUSH_RETRIES) {
              console.warn(`[OfflineSync] delete failed for note ${entry.note_id}:`, fetchErr);
              await incrementPendingSyncRetry(entry.note_id);
              const retryCount = await getPendingSyncRetryCount(entry.note_id);
              if (retryCount >= MAX_RETRY_ATTEMPTS) {
                console.warn(`[OfflineSync] clearing delete entry for note ${entry.note_id}: max retries exceeded (network error)`);
                await clearPendingSync(entry.note_id);
              }
              failed++;
            }
            // else: transient network error, loop continues with exponential backoff
          } finally {
            clearTimeout(timer);
          }
        }

        if (shouldStopFlush) break; // stop processing remaining pending entries
        continue;
      }

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

      let shouldStopFlush = false;

      for (let attempt = 0; attempt <= FLUSH_RETRIES; attempt++) {
        if (attempt > 0) {
          // Exponential backoff: 1s, 2s
          await new Promise((r) => setTimeout(r, RETRY_BASE_MS * Math.pow(2, attempt - 1)));
        }

        const timeoutController = new AbortController();
        const timer = setTimeout(() => timeoutController.abort(), 10000);
        try {
          const res = await fetch(`${url}/api/notes/${encodeURIComponent(entry.note_id)}`, {
            method: 'PUT',
            headers,
            body: JSON.stringify({ title: note.title, content: note.content }),
            signal: timeoutController.signal,
          });

          if (res.ok) {
            await clearPendingSync(entry.note_id);
            synced++;
            break; // success
          }

          if (res.status === 429) {
            console.warn(`[OfflineSync] rate limited for note ${entry.note_id}: ${res.status}, pausing flush`);
            failed++;
            shouldStopFlush = true;
            break; // rate limited, stop entire flush
          }

          if (res.status === 408) {
            // Request Timeout: transient, retry on next flush (#2501)
            console.warn(`[OfflineSync] request timeout for note ${entry.note_id}: ${res.status}, will retry`);
            await incrementPendingSyncRetry(entry.note_id);
            const retryCount = await getPendingSyncRetryCount(entry.note_id);
            if (retryCount >= MAX_RETRY_ATTEMPTS) {
              console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: max retries exceeded (408)`);
              await clearPendingSync(entry.note_id);
            }
            failed++;
            break;
          }

          if (res.status >= 400 && res.status < 500) {
            // Client error (4xx): clear entry, won't succeed on retry
            console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: client error ${res.status}`);
            await clearPendingSync(entry.note_id);
            failed++;
            break; // client error, won't succeed on retry
          }

          // 5xx or unknown server error — retry if attempts remain
          if (attempt >= FLUSH_RETRIES) {
            // Last attempt failed — fall back to cross-cycle retry
            await incrementPendingSyncRetry(entry.note_id);
            const retryCount = await getPendingSyncRetryCount(entry.note_id);
            if (retryCount >= MAX_RETRY_ATTEMPTS) {
              console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: max retries exceeded`);
              await clearPendingSync(entry.note_id);
            }
            failed++;
          }
          // else: transient 5xx, loop continues with exponential backoff
        } catch (fetchErr) {
          if (attempt >= FLUSH_RETRIES) {
            console.warn(`[OfflineSync] PUT failed for note ${entry.note_id}:`, fetchErr);
            await incrementPendingSyncRetry(entry.note_id);
            const retryCount = await getPendingSyncRetryCount(entry.note_id);
            if (retryCount >= MAX_RETRY_ATTEMPTS) {
              console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: max retries exceeded (network error)`);
              await clearPendingSync(entry.note_id);
            }
            failed++;
          }
          // else: transient network error, loop continues with exponential backoff
        } finally {
          clearTimeout(timer);
        }
      }

      if (shouldStopFlush) break;
    } catch (e) {
      console.warn('[OfflineSync] flush failed for note:', entry.note_id, e);
      await incrementPendingSyncRetry(entry.note_id);
      const retryCount = await getPendingSyncRetryCount(entry.note_id);
      if (retryCount >= MAX_RETRY_ATTEMPTS) {
        console.warn(`[OfflineSync] clearing entry for note ${entry.note_id}: max retries exceeded (network error)`);
        await clearPendingSync(entry.note_id);
      }
      failed++;
      // Continue processing remaining entries in the queue
      continue;
    }
  }

  return { synced, failed };
}
