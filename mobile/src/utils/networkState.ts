/**
 * Network state detection for offline mode (#1220).
 *
 * Uses navigator.onLine + fetch health check to detect connectivity.
 * No external dependencies required.
 */

import { useEffect, useState, useCallback, useRef } from 'react';

/** Typed accessor for globalThis event listeners (browser/RN). */
interface GlobalEventBus {
  addEventListener(type: string, listener: () => void): void;
  removeEventListener(type: string, listener: () => void): void;
}

/** Simple online/offline state hook. */
export function useNetworkState(): { isOnline: boolean; checkConnection: () => Promise<boolean> } {
  const [isOnline, setIsOnline] = useState(true);
  const abortRef = useRef<AbortController | null>(null);
  const generationRef = useRef(0);

  const checkConnection = useCallback(async (): Promise<boolean> => {
    const gen = ++generationRef.current;
    try {
      // Quick HEAD request to a reliable endpoint
      const timeoutController = new AbortController();
      abortRef.current = timeoutController;
      const timer = setTimeout(() => timeoutController.abort(), 3000);
      try {
        const res = await fetch('https://www.gstatic.com/generate_204', {
          method: 'HEAD',
          signal: timeoutController.signal,
        });
        const online = res.ok;
        // Only apply result if no newer fetch was started (avoids stale
        // result overwriting event-driven state changes).
        if (gen === generationRef.current) {
          setIsOnline(online);
        }
        return online;
      } finally {
        clearTimeout(timer);
        abortRef.current = null;
      }
    } catch (e) {
      console.warn('[NetworkState] checkConnection failed:', e);
      if (gen === generationRef.current) {
        setIsOnline(false);
      }
      return false;
    }
  }, []);

  useEffect(() => {
    const handleOnline = () => setIsOnline(true);
    const handleOffline = () => setIsOnline(false);

    // Listen to browser/RN online/offline events
    const win = globalThis as Partial<GlobalEventBus>;
    if (win.addEventListener) {
      win.addEventListener('online', handleOnline);
      win.addEventListener('offline', handleOffline);
    }

    // #1448: Initial connectivity check — actively verify network reachability
    // instead of relying solely on navigator.onLine (unreliable in RN).
    if (typeof navigator !== 'undefined' && navigator.onLine === false) {
      setIsOnline(false);
    } else {
      checkConnection();
    }

    return () => {
      abortRef.current?.abort();
      abortRef.current = null;
      if (win.removeEventListener) {
        win.removeEventListener('online', handleOnline);
        win.removeEventListener('offline', handleOffline);
      }
    };
  }, [checkConnection]);

  return { isOnline, checkConnection };
}
