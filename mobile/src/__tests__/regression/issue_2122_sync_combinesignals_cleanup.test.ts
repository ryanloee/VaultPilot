/**
 * Regression test for combineSignals cleanup leak (#2122).
 *
 * Bug: doSync() declared `const { cleanup } = combineSignals(...)` inside the
 * `try` block and only called `cleanup()` on the success path. When `fetch`
 * threw (network error / timeout / abort), execution entered the `catch` block,
 * where `cleanup` was out of scope (block-scoped) and thus never invoked. The
 * `abort` event listeners that combineSignals registered on the long-lived
 * outer sync signal were never removed, so each failed retry leaked one
 * listener. Syncing hundreds of notes accumulated hundreds/thousands of
 * listeners on the same signal.
 *
 * Fix: move the combineSignals() call outside the try block and call cleanup
 * in a `finally`, so listeners are removed on every path (success/error/abort).
 *
 * This test verifies the behavior by tracking `addEventListener('abort', ...)`
 * vs `removeEventListener('abort', ...)` balance across a sync where the note
 * detail fetch keeps rejecting (exercising the catch + retry path).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer } from '../../services/sync';

jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
  getNoteTimestamps: jest.fn(),
  getPendingSyncs: jest.fn().mockResolvedValue([]),
}));

const mockGetNoteTimestamps = require('../../db').getNoteTimestamps as jest.MockedFunction<any>;

const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  await AsyncStorage.clear();
  mockFetch.mockReset();
  mockGetNoteTimestamps.mockResolvedValue([]);
});

describe('combineSignals cleanup leak on fetch error (#2122)', () => {
  it('detail fetch network errors do not leak abort listeners on the sync signal', async () => {
    (
      AsyncStorage.getItem as jest.Mock
    )
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockGetNoteTimestamps.mockResolvedValue([]);

    // List endpoint succeeds with one note to fetch in detail.
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          notes: [{ id: 'note-1', title: 'Leak', updated_at: '2026-06-01T00:00:00Z' }],
          total: 1,
        }),
    });
    // Detail endpoint always rejects → exercises catch path on every retry.
    // MAX_RETRIES = 2 → 3 attempts, each registering (and pre-fix: NOT removing) listeners.
    mockFetch.mockRejectedValue(new Error('Network error'));

    // Track abort-listener add/remove balance via EventTarget (AbortSignal extends it).
    const added = new Map<AbortSignal, Set<EventListenerOrEventListenerObject>>();
    const removed = new Map<AbortSignal, Set<EventListenerOrEventListenerObject>>();

    const proto = EventTarget.prototype;
    const origAdd = proto.addEventListener;
    const origRemove = proto.removeEventListener;

    proto.addEventListener = function (
      this: EventTarget,
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: boolean | AddEventListenerOptions,
    ) {
      if (type === 'abort' && listener && this instanceof AbortSignal) {
        const sig = this as AbortSignal;
        if (!added.has(sig)) added.set(sig, new Set());
        added.get(sig)!.add(listener as EventListener);
      }
      return origAdd.call(this, type as any, listener as any, options as any);
    } as any;

    proto.removeEventListener = function (
      this: EventTarget,
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: boolean | EventListenerOptions,
    ) {
      if (type === 'abort' && listener && this instanceof AbortSignal) {
        const sig = this as AbortSignal;
        if (!removed.has(sig)) removed.set(sig, new Set());
        removed.get(sig)!.add(listener as EventListener);
      }
      return origRemove.call(this, type as any, listener as any, options as any);
    } as any;

    try {
      const result = await syncNotesFromServer();
      // Sanity: the note failed to fetch (error path taken).
      expect(result.errors).toBe(1);
      expect(result.imported).toBe(0);

      // Core assertion: every abort listener added to any AbortSignal must have
      // been removed (no leak). Before the fix, the catch path skipped cleanup,
      // so added > removed.
      for (const [sig, listeners] of added) {
        const removedSet = removed.get(sig) ?? new Set<EventListener>();
        for (const handler of listeners) {
          expect(removedSet.has(handler)).toBe(true);
        }
      }

      // Aggregate counts must match exactly.
      let totalAdded = 0;
      let totalRemoved = 0;
      for (const set of added.values()) totalAdded += set.size;
      for (const set of removed.values()) totalRemoved += set.size;
      expect(totalRemoved).toBe(totalAdded);
    } finally {
      proto.addEventListener = origAdd;
      proto.removeEventListener = origRemove;
    }
  });
});
