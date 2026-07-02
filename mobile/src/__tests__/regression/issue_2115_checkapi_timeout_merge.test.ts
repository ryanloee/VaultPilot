/**
 * Regression test for #2329 (was #2115): checkApi must keep its 8s timeout even
 * when the caller supplies an external AbortSignal, and must not use the
 * non-Hermes-compatible AbortSignal.timeout() / AbortSignal.any() APIs.
 *
 * Bug (#2329): AbortSignal.timeout() / AbortSignal.any() are unavailable in
 * React Native Hermes (Hermes 0.12 / RN 0.73), causing TypeError crashes.
 * Fix: use setTimeout + manual AbortController to combine timeout + user signal.
 *
 * Bug (#2115): Previously used `signal ?? AbortSignal.timeout(8000)` with ??,
 * so when SettingsScreen.testConnection passed its own (non-timeout) AbortSignal,
 * the 8s timeout was short-circuited entirely. Against a host that accepts the TCP
 * connection but never returns an HTTP response, fetch neither resolved nor
 * rejected, so the ActivityIndicator spun forever.
 */

import { checkApi } from '../../api/client';

// ── Mock fetch: emulates a hanging host. Rejects with AbortError as soon as the
//    request's signal aborts; otherwise never settles. ────────────────────────
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
    // Never resolves otherwise — emulates a non-responsive host.
  });
});
(globalThis as any).fetch = mockFetch;

beforeEach(() => {
  mockFetch.mockClear();
});

describe('issue #2329 — checkApi uses Hermes-compatible timeout', () => {
  it('setTimeout is created with the correct delay when an external signal is passed', async () => {
    const timeoutSpy = jest.spyOn(globalThis, 'setTimeout');

    // External controller (like SettingsScreen's), NOT aborted — must not
    // suppress the built-in timeout. Don't await (hanging host).
    checkApi({
      apiBase: 'https://hanging.example.com',
      apiKey: 'sk-test',
      signal: new AbortController().signal,
    });

    // setTimeout must have been called with the timeout value.
    expect(timeoutSpy).toHaveBeenCalledWith(expect.any(Function), 8000);
    timeoutSpy.mockRestore();
  });

  it('8s timeout firing ends the request even though the external signal never aborts', async () => {
    jest.useFakeTimers();

    // External signal (SettingsScreen's) is never aborted — user just stares at spinner.
    const external = new AbortController();
    const promise = checkApi({
      apiBase: 'https://hanging.example.com',
      apiKey: 'sk-test',
      apiFormat: 'openai',
      signal: external.signal,
    });

    // Fetch received an abortable signal.
    const fetchSignal = (mockFetch.mock.calls[0][1] as RequestInit).signal as AbortSignal;
    expect(fetchSignal.aborted).toBe(false);

    // Simulate the 8s timer elapsing by running pending timers.
    jest.advanceTimersByTime(8000);

    // Allow microtasks (the fetch rejection + checkApi catch) to flush.
    await promise;

    // The external signal must remain un-aborted (timeout fired, not the user).
    expect(external.signal.aborted).toBe(false);

    jest.useRealTimers();
  });

  it('external abort still wins when it fires before the timeout', async () => {
    jest.useFakeTimers();

    const external = new AbortController();
    const promise = checkApi({
      apiBase: 'https://hanging.example.com',
      apiKey: 'sk-test',
      apiFormat: 'openai',
      signal: external.signal,
    });

    const fetchSignal = (mockFetch.mock.calls[0][1] as RequestInit).signal as AbortSignal;
    // User navigates away / taps again → external abort propagates to fetch.
    external.abort();
    expect(fetchSignal.aborted).toBe(true);

    const res = await promise;
    expect(res.ok).toBe(false);

    jest.useRealTimers();
  });

  it('still applies the 8s timeout when no external signal is provided', async () => {
    const timeoutSpy = jest.spyOn(globalThis, 'setTimeout');
    // Fire and forget — we only assert setTimeout is called with the right delay.
    checkApi({ apiBase: 'https://hanging.example.com', apiKey: 'sk-test' });
    expect(timeoutSpy).toHaveBeenCalledWith(expect.any(Function), 8000);
    timeoutSpy.mockRestore();
  });

  it('does not use AbortSignal.timeout or AbortSignal.any (Hermes compat)', () => {
    const timeoutSpy = jest.spyOn(AbortSignal, 'timeout');
    const anySpy = jest.spyOn(AbortSignal, 'any');

    checkApi({ apiBase: 'https://hanging.example.com', apiKey: 'sk-test' });

    expect(timeoutSpy).not.toHaveBeenCalled();
    expect(anySpy).not.toHaveBeenCalled();

    timeoutSpy.mockRestore();
    anySpy.mockRestore();
  });
});
