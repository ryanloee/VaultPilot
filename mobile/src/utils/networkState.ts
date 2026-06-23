/**
 * Network state detection for offline mode (#1220).
 *
 * Uses navigator.onLine + fetch health check to detect connectivity.
 * No external dependencies required.
 */

import { useEffect, useState, useCallback } from 'react';

/** Simple online/offline state hook. */
export function useNetworkState(): { isOnline: boolean; checkConnection: () => Promise<boolean> } {
  const [isOnline, setIsOnline] = useState(true);

  const checkConnection = useCallback(async (): Promise<boolean> => {
    try {
      // Quick HEAD request to a reliable endpoint
      const res = await fetch('https://www.gstatic.com/generate_204', {
        method: 'HEAD',
        signal: AbortSignal.timeout(3000),
      });
      const online = res.ok;
      setIsOnline(online);
      return online;
    } catch (e) {
      console.warn('[NetworkState] checkConnection failed:', e);
      setIsOnline(false);
      return false;
    }
  }, []);

  useEffect(() => {
    const handleOnline = () => setIsOnline(true);
    const handleOffline = () => setIsOnline(false);

    // Listen to browser/RN online/offline events
    const win = globalThis as any;
    if (win.addEventListener) {
      win.addEventListener('online', handleOnline);
      win.addEventListener('offline', handleOffline);
    }

    // Initial check
    if (typeof navigator !== 'undefined' && navigator.onLine === false) {
      setIsOnline(false);
    }

    return () => {
      if (win.removeEventListener) {
        win.removeEventListener('online', handleOnline);
        win.removeEventListener('offline', handleOffline);
      }
    };
  }, []);

  return { isOnline, checkConnection };
}
