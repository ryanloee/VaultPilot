/**
 * Network state detection for offline mode (#1220).
 *
 * Uses navigator.onLine + fetch health check to detect connectivity.
 * No external dependencies required.
 */

import { useEffect, useState, useCallback, useRef } from 'react';
import { pingBackend } from '../services/sync';

/** Typed accessor for globalThis event listeners (browser/RN). */
interface GlobalEventBus {
  addEventListener(type: string, listener: () => void): void;
  removeEventListener(type: string, listener: () => void): void;
}

/**
 * Probe real network reachability.
 *
 * #3868: 优先探测已配置后端的 /health（与同步同一网络路径，国内可达）；
 * 未配置后端（pingBackend 返回 false）时才回退到公共端点 gstatic.com，
 * 判断通用网络连通性。gstatic.com（Google）在大陆不可达，因此仅在
 * 后端未配置时使用它，避免误判离线。
 */
export async function probeConnectivity(): Promise<boolean> {
  const backendReachable = await pingBackend();
  if (backendReachable) return true;
  const timeoutController = new AbortController();
  const timer = setTimeout(() => timeoutController.abort(), 3000);
  try {
    const res = await fetch('https://www.gstatic.com/generate_204', {
      method: 'HEAD',
      signal: timeoutController.signal,
    });
    return res.ok;
  } catch (e) {
    console.warn('[NetworkState] fallback probe failed:', e);
    return false;
  } finally {
    clearTimeout(timer);
  }
}

/** Simple online/offline state hook. */
export function useNetworkState(): { isOnline: boolean; checkConnection: () => Promise<boolean> } {
  const [isOnline, setIsOnline] = useState(true);
  const generationRef = useRef(0);

  const checkConnection = useCallback(async (): Promise<boolean> => {
    const gen = ++generationRef.current;
    try {
      const online = await probeConnectivity();
      // Only apply result if no newer fetch was started (avoids stale
      // result overwriting event-driven state changes).
      if (gen === generationRef.current) {
        setIsOnline(online);
      }
      return online;
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
      if (win.removeEventListener) {
        win.removeEventListener('online', handleOnline);
        win.removeEventListener('offline', handleOffline);
      }
    };
  }, [checkConnection]);

  return { isOnline, checkConnection };
}
