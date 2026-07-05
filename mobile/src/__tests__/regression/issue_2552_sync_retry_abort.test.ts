/**
 * Regression test for sync retry delay interruptibility (#2552).
 *
 * Bug: Both retry loops (list fetch and detail fetch) used a raw
 *   await new Promise(r => setTimeout(r, delay))
 * that completely ignored the abort signal. If the server returned a long
 * Retry-After, or the overall 5-minute timeout fired, the sync would block
 * for the full delay before checking the abort signal again.
 *
 * Fix: raceDelayOrAbort() resolves immediately when the signal aborts,
 * ensuring the delay never blocks sync exit past the timeout or user cancel.
 */

import { raceDelayOrAbort } from '../../services/sync';

describe('raceDelayOrAbort (#2552)', () => {
  jest.useFakeTimers();

  beforeEach(() => {
    jest.clearAllTimers();
  });

  afterAll(() => {
    jest.useRealTimers();
  });

  it('resolves after delay when signal does not abort', async () => {
    const controller = new AbortController();
    const spy = jest.fn();
    raceDelayOrAbort(controller.signal, 5000).then(spy);

    // Not resolved yet
    await Promise.resolve();
    expect(spy).not.toHaveBeenCalled();

    jest.advanceTimersByTime(5000);
    await Promise.resolve();
    expect(spy).toHaveBeenCalled();
  });

  it('resolves immediately when signal is already aborted', async () => {
    const controller = new AbortController();
    controller.abort();
    // Should resolve without any timer advancement
    await raceDelayOrAbort(controller.signal, 60000);
    // If we reach here, it resolved — pass
    expect(true).toBe(true);
  });

  it('resolves early when signal aborts during delay', async () => {
    const controller = new AbortController();
    const spy = jest.fn();
    raceDelayOrAbort(controller.signal, 60000).then(spy);

    await Promise.resolve();
    expect(spy).not.toHaveBeenCalled();

    // Abort after 1s of a 60s delay
    jest.advanceTimersByTime(1000);
    controller.abort();
    await Promise.resolve();
    expect(spy).toHaveBeenCalled();

    // The full delay has NOT elapsed
    expect(jest.getTimerCount()).toBe(0); // timer was cleaned up
  });

  it('cleans up the abort listener after normal completion', async () => {
    const controller = new AbortController();
    const removeSpy = jest.spyOn(controller.signal, 'removeEventListener');

    // Set up the promise first, then advance fake timers to trigger completion
    const promise = raceDelayOrAbort(controller.signal, 3000);
    jest.advanceTimersByTime(3000);
    await promise;

    // Listener should have been removed during cleanup
    expect(removeSpy).toHaveBeenCalled();
    removeSpy.mockRestore();
  });
});
