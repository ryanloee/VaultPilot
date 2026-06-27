/**
 * Regression test for #2115: checkApi must keep its 8s timeout even when the
 * caller supplies an external AbortSignal.
 *
 * Bug: `const effectiveSignal = signal ?? AbortSignal.timeout(8000);` used `??`,
 * so when SettingsScreen.testConnection passed its own (non-timeout) AbortSignal,
 * the 8s timeout was short-circuited entirely. Against a host that accepts the TCP
 * connection but never returns an HTTP response, fetch neither resolved nor
 * rejected, so the ActivityIndicator spun forever.
 *
 * Fix: merge the external signal with the built-in timeout via AbortSignal.any.
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

describe('issue #2115 — checkApi merges external signal with 8s timeout', () => {
  it('AbortSignal.timeout is still created when an external signal is passed', async () => {
    const timeoutSpy = jest.spyOn(AbortSignal, 'timeout');

    // External controller (like SettingsScreen's), NOT aborted — must not
    // suppress the built-in timeout. Don't await (hanging host).
    checkApi({
      apiBase: 'https://hanging.example.com',
      apiKey: 'sk-test',
      signal: new AbortController().signal,
    });

    // With the buggy `??`, AbortSignal.timeout would never be called here.
    expect(timeoutSpy).toHaveBeenCalledWith(8000);
    timeoutSpy.mockRestore();
  });

  it('8s timeout firing ends the request even though the external signal never aborts', async () => {
    // Stub AbortSignal.timeout with a real, controllable signal so we can
    // deterministically "fire" the 8s timer without waiting.
    const timeoutController = new AbortController();
    jest.spyOn(AbortSignal, 'timeout').mockReturnValue(timeoutController.signal);

    // External signal (SettingsScreen's) is never aborted — user just stares at spinner.
    const external = new AbortController();
    const promise = checkApi({
      apiBase: 'https://hanging.example.com',
      apiKey: 'sk-test',
      apiFormat: 'openai',
      signal: external.signal,
    });

    // Fetch received a merged signal (AbortSignal.any of timeout + external).
    const fetchSignal = (mockFetch.mock.calls[0][1] as RequestInit).signal as AbortSignal;
    expect(fetchSignal.aborted).toBe(false);

    // Simulate the 8s timer elapsing.
    timeoutController.abort();

    // The merged signal must now be aborted → fetch rejects → checkApi returns
    // an error instead of hanging forever.
    const res = await promise;
    expect(res.ok).toBe(false);
    expect(res.error).toBeTruthy();
    // The external signal must remain un-aborted (timeout fired, not the user).
    expect(external.signal.aborted).toBe(false);

    jest.restoreAllMocks();
  });

  it('external abort still wins when it fires before the timeout', async () => {
    const timeoutController = new AbortController();
    jest.spyOn(AbortSignal, 'timeout').mockReturnValue(timeoutController.signal);

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

    jest.restoreAllMocks();
  });

  it('still applies the 8s timeout when no external signal is provided', async () => {
    const timeoutSpy = jest.spyOn(AbortSignal, 'timeout');
    // Fire and forget — we only assert the timeout signal is built.
    checkApi({ apiBase: 'https://hanging.example.com', apiKey: 'sk-test' });
    expect(timeoutSpy).toHaveBeenCalledWith(8000);
    timeoutSpy.mockRestore();
  });
});
