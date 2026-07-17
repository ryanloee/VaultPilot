/**
 * Regression test for #3036:
 * chatAnthropic must convert fetch-phase AbortError (the 2-minute timeout
 * firing before the ReadableStream is returned) into a friendly Chinese
 * message '请求超时（2 分钟），请检查网络或服务端状态', exactly like chatOpenAI.
 *
 * Previously chatAnthropic only had try...finally (no catch), so a timeout
 * during fetch/retry left a raw DOMException('Aborted','AbortError') reaching
 * the UI, inconsistent with the OpenAI path which already translated it.
 */
import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';

jest.mock('@react-native-async-storage/async-storage', () => ({
  getItem: jest.fn(),
  setItem: jest.fn(),
}));

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
}));

import { chat, invalidateSettingsCache } from '../../api/client';

// Flush the microtask queue a few times so async getSettings / fetch settle
// while fake timers are active. (Needed because jest.useFakeTimers does not
// auto-flush promise microtasks.)
function flushMicrotasks(n = 5) {
  let p = Promise.resolve();
  for (let i = 0; i < n; i++) p = p.then(() => undefined);
  return p;
}

beforeEach(() => {
  jest.restoreAllMocks();
  (AsyncStorage.getItem as jest.Mock).mockImplementation((key: string) => {
    if (key === 'cfg_api_base') return Promise.resolve('https://api.anthropic.com');
    if (key === 'cfg_api_format') return Promise.resolve('anthropic');
    if (key === 'cfg_model') return Promise.resolve('claude-3-5-sonnet-20241022');
    return Promise.resolve(null);
  });
  (SecureStore.getItemAsync as jest.Mock).mockResolvedValue('sk-test-key');
  invalidateSettingsCache();
});

describe('issue #3036 — chatAnthropic converts fetch-phase timeout to friendly message', () => {
  it('throws friendly Chinese message (not raw AbortError) when the internal timeout fires', async () => {
    jest.useFakeTimers();

    // Mock fetch: hanging host — rejects with AbortError once the signal aborts.
    const mockFetch = jest.fn((_url: string, init?: RequestInit) => {
      return new Promise((_resolve, reject) => {
        const sig = init?.signal as AbortSignal | undefined;
        const onAbort = () => {
          const err = new Error('aborted');
          err.name = 'AbortError';
          reject(err);
        };
        if (sig) {
          if (sig.aborted) onAbort();
          else sig.addEventListener('abort', onAbort, { once: true });
        }
      });
    });
    (globalThis as any).fetch = mockFetch;

    // No external signal — only the internal 2-minute timer aborts.
    const promise = chat([{ role: 'user', content: 'hi' }]);

    // Let getSettings() resolve so the first fetch is issued.
    await flushMicrotasks();
    expect(mockFetch).toHaveBeenCalledTimes(1);

    const fetchSignal = (mockFetch.mock.calls[0][1] as RequestInit).signal as AbortSignal;
    expect(fetchSignal.aborted).toBe(false);

    // Advance past CHAT_TIMEOUT_MS (120s) so the internal timer aborts.
    jest.advanceTimersByTime(120000);

    // Let the fetch rejection + chatAnthropic catch propagate.
    let caught: unknown;
    try {
      await promise;
    } catch (e) {
      caught = e;
    }

    jest.useRealTimers();

    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).toBe('请求超时（2 分钟），请检查网络或服务端状态');
    // Must NOT be a raw AbortError — that was the bug.
    expect((caught as Error).name).not.toBe('AbortError');
  });

  it('still re-throws AbortError (not the timeout message) when the user-supplied signal aborts', async () => {
    jest.useFakeTimers();

    const mockFetch = jest.fn((_url: string, init?: RequestInit) => {
      return new Promise((_resolve, reject) => {
        const sig = init?.signal as AbortSignal | undefined;
        const onAbort = () => {
          const err = new Error('aborted');
          err.name = 'AbortError';
          reject(err);
        };
        if (sig) {
          if (sig.aborted) onAbort();
          else sig.addEventListener('abort', onAbort, { once: true });
        }
      });
    });
    (globalThis as any).fetch = mockFetch;

    // External signal aborts → the timeout did NOT fire, so the AbortError
    // must propagate directly, NOT be converted to the timeout message.
    const external = new AbortController();
    const promise = chat([{ role: 'user', content: 'hi' }], external.signal);

    await flushMicrotasks();
    external.abort();

    let caught: unknown;
    try {
      await promise;
    } catch (e) {
      caught = e;
    }

    jest.useRealTimers();

    expect(caught).toBeDefined();
    // The timeout message must NOT appear — the user aborted, not the timer.
    if (caught instanceof Error) {
      expect(caught.message).not.toBe('请求超时（2 分钟），请检查网络或服务端状态');
    }
  });

  it('preserves non-AbortError failures untouched (no timeout conversion)', async () => {
    jest.useFakeTimers();

    // Every fetch rejects with a plain network error (not AbortError) → retries
    // exhaust → chatAnthropic throws '请求失败，已重试多次', not the timeout msg.
    (globalThis as any).fetch = jest.fn().mockImplementation(() => {
      return Promise.reject(new Error('network down'));
    });

    const promise = chat([{ role: 'user', content: 'hi' }]);

    // Advance through the retry delays (RETRY_BASE_MS * 2^0 + * 2^1 = 1s + 2s).
    await flushMicrotasks();
    jest.advanceTimersByTime(1000);
    await flushMicrotasks();
    jest.advanceTimersByTime(2000);
    await flushMicrotasks();

    let caught: unknown;
    try {
      await promise;
    } catch (e) {
      caught = e;
    }

    jest.useRealTimers();

    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).not.toContain('请求超时');
  });
});
