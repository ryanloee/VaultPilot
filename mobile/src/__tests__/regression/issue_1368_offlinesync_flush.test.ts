/**
 * Regression tests for offlineSync flushPendingSyncs (#1368, #1596).
 *
 * Verifies:
 * 1. Empty server config → returns {synced: 0, failed: 0}
 * 2. Empty pending queue → returns {synced: 0, failed: 0}
 * 3. All syncs succeed → correct counts
 * 4. Deleted note locally → clears pending, counts as synced
 * 5. Server returns non-ok → increments failed
 * 6. Network error mid-flush → continues processing, returns partial counts
 * 7. 4xx error → clears entry from queue, counts as failed
 * 8. 5xx error → increments retry count
 * 9. 5xx error max retries → clears entry from queue
 */

const mockGetServerConfig = jest.fn();
const mockGetPendingSyncs = jest.fn();
const mockGetNote = jest.fn();
const mockClearPendingSync = jest.fn();
const mockGetPendingSyncCount = jest.fn().mockResolvedValue(0);
const mockQueuePendingSync = jest.fn();
const mockGetPendingSync = jest.fn();
const mockIncrementPendingSyncRetry = jest.fn();
const mockGetPendingSyncRetryCount = jest.fn();
const mockGetNoteTags = jest.fn();

jest.mock('../../services/sync', () => ({
  getServerConfig: mockGetServerConfig,
}));

jest.mock('../../db', () => ({
  getPendingSyncs: mockGetPendingSyncs,
  getNote: mockGetNote,
  clearPendingSync: mockClearPendingSync,
  getPendingSyncCount: mockGetPendingSyncCount,
  queuePendingSync: mockQueuePendingSync,
  getPendingSync: mockGetPendingSync,
  incrementPendingSyncRetry: mockIncrementPendingSyncRetry,
  getPendingSyncRetryCount: mockGetPendingSyncRetryCount,
  getNoteTags: mockGetNoteTags,
}));

jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: jest.fn().mockResolvedValue({
    execAsync: jest.fn(),
    getAllAsync: jest.fn().mockResolvedValue([]),
    getFirstAsync: jest.fn().mockResolvedValue(null),
    runAsync: jest.fn(),
    withTransactionAsync: jest.fn().mockImplementation(async (fn: () => Promise<void>) => fn()),
  }),
}));

// Mock global fetch
const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

// AbortSignal is no longer mocked — code uses AbortController directly (fix #2329).

import { flushPendingSyncs } from '../../utils/offlineSync';

describe('flushPendingSyncs', () => {
  jest.setTimeout(30000); // retry backoff: 1s+2s per 5xx entry
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetServerConfig.mockResolvedValue({ url: 'http://localhost:8080', token: 'tok123' });
    mockGetPendingSyncs.mockResolvedValue([]);
    mockGetNote.mockResolvedValue(null);
    mockClearPendingSync.mockResolvedValue(undefined);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 1, note_id: noteId, action: 'update', retry_count: 0 })
    );
    mockIncrementPendingSyncRetry.mockResolvedValue(undefined);
    mockGetPendingSyncRetryCount.mockResolvedValue(0);
    mockGetNoteTags.mockResolvedValue([]);
  });

  it('returns zeros when no server URL configured', async () => {
    mockGetServerConfig.mockResolvedValue({ url: '', token: '' });
    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 0 });
  });

  it('returns zeros when pending queue is empty', async () => {
    mockGetPendingSyncs.mockResolvedValue([]);
    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 0 });
  });

  it('syncs all notes successfully', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
    ]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T1', content: 'C1' });
    mockFetch.mockResolvedValue({ ok: true });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 2, failed: 0 });
    expect(mockClearPendingSync).toHaveBeenCalledTimes(2);
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it('handles deleted note locally', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'deleted1' }]);
    mockGetNote.mockResolvedValue(null); // note deleted

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 1, failed: 0 });
    expect(mockClearPendingSync).toHaveBeenCalledWith('deleted1');
    expect(mockFetch).not.toHaveBeenCalled();
  });

  // #2522: Stale snapshot with 'update' action while DB has 'delete' action
  it('sends DELETE to server when stale snapshot had update but DB action is delete', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    // Note was deleted locally after snapshot, so getNote returns null
    mockGetNote.mockResolvedValue(null);
    // The DB entry now has action='delete' but snapshot had stale 'update'
    mockGetPendingSync.mockResolvedValue({ id: 1, note_id: 'n1', action: 'delete', retry_count: 0 });
    mockFetch.mockResolvedValue({ ok: true });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 1, failed: 0 });
    // Must send DELETE to server to prevent data loss
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockFetch.mock.calls[0][1].method).toBe('DELETE');
    expect(mockClearPendingSync).toHaveBeenCalledWith('n1');
  });

  // #2522: Entry cleared between snapshot and re-query — skip gracefully
  it('skips entry that was cleared between snapshot and re-query', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetPendingSync.mockResolvedValue(null); // entry already deleted

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 0 });
    expect(mockClearPendingSync).not.toHaveBeenCalled();
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('counts server error as failed', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 500 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('n1');
  });

  it('continues flushing after network error and returns partial counts', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
      { note_id: 'n3' },
    ]);
    // First note succeeds, second throws network error, third succeeds
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    // n2's network error is retried 3 times (FLUSH_RETRIES=2 → loop 0..2)
    mockFetch
      .mockResolvedValueOnce({ ok: true })                              // n1
      .mockRejectedValueOnce(new Error('Network request failed'))      // n2 attempt 0
      .mockRejectedValueOnce(new Error('Network request failed'))      // n2 attempt 1
      .mockRejectedValueOnce(new Error('Network request failed'))      // n2 attempt 2
      .mockResolvedValue({ ok: true });                                 // n3+

    const result = await flushPendingSyncs();
    expect(result.synced).toBe(2);   // n1 + n3
    expect(result.failed).toBe(1);   // n2 only
    // Should have continued after error — n3 processed too (n1×1 + n2×3 + n3×1 = 5 calls)
    expect(mockFetch).toHaveBeenCalledTimes(5);
  });

  it('includes Authorization header when token is set', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: true });

    await flushPendingSyncs();
    const callHeaders = mockFetch.mock.calls[0][1]?.headers as Record<string, string>;
    expect(callHeaders['Authorization']).toBe('Bearer tok123');
  });

  it('omits Authorization header when token is empty', async () => {
    mockGetServerConfig.mockResolvedValue({ url: 'http://localhost:8080', token: '' });
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: true });

    await flushPendingSyncs();
    const callHeaders = mockFetch.mock.calls[0][1]?.headers as Record<string, string>;
    expect(callHeaders['Authorization']).toBeUndefined();
  });

  // ── #1449: Additional edge case tests ──

  it('continues to next note after server error (non-network failure)', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
      { note_id: 'n3' },
    ]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    // n1's 500 is retried 3 times before giving up
    mockFetch
      .mockResolvedValueOnce({ ok: false, status: 500 })   // n1 attempt 0
      .mockResolvedValueOnce({ ok: false, status: 500 })   // n1 attempt 1
      .mockResolvedValueOnce({ ok: false, status: 500 })   // n1 attempt 2
      .mockResolvedValueOnce({ ok: true })                 // n2
      .mockResolvedValue({ ok: true });                    // n3+

    const result = await flushPendingSyncs();
    // n1 failed (server error), n2 and n3 synced
    expect(result.synced).toBe(2);
    expect(result.failed).toBe(1);
    expect(mockFetch).toHaveBeenCalledTimes(5);  // n1×3 + n2×1 + n3×1
  });

  it('handles all notes deleted locally', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'deleted1' },
      { note_id: 'deleted2' },
      { note_id: 'deleted3' },
    ]);
    mockGetNote.mockResolvedValue(null); // all deleted

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 3, failed: 0 });
    expect(mockClearPendingSync).toHaveBeenCalledTimes(3);
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('passes AbortController signal to fetch (timeout via setTimeout)', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: true });

    await flushPendingSyncs();
    const callSignal = mockFetch.mock.calls[0][1]?.signal;
    expect(callSignal).toBeDefined();
    expect(callSignal instanceof AbortSignal).toBe(true);
    expect(callSignal.aborted).toBe(false);
  });

  it('sends correct PUT body with title and content', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'note-xyz' }]);
    mockGetNote.mockResolvedValue({ id: 'note-xyz', title: 'My Title', content: 'My Content' });
    mockFetch.mockResolvedValue({ ok: true });

    await flushPendingSyncs();
    const fetchBody = JSON.parse(mockFetch.mock.calls[0][1].body);
    expect(fetchBody).toEqual({ title: 'My Title', content: 'My Content' });
  });

  it('uses PUT method for sync', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: true });

    await flushPendingSyncs();
    expect(mockFetch.mock.calls[0][1].method).toBe('PUT');
  });

  // ── #1596: Fix for non-OK response handling ──

  it('clears entry on 4xx client error', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 404 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockClearPendingSync).toHaveBeenCalledWith('n1');
    expect(mockIncrementPendingSyncRetry).not.toHaveBeenCalled();
  });

  it('clears entry on 400 bad request', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 400 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockClearPendingSync).toHaveBeenCalledWith('n1');
  });

  it('increments retry count on 5xx server error', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 500 });
    mockGetPendingSyncRetryCount.mockResolvedValue(1);

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('n1');
    expect(mockGetPendingSyncRetryCount).toHaveBeenCalledWith('n1');
    expect(mockClearPendingSync).not.toHaveBeenCalled();
  });

  it('clears entry after max retry attempts exceeded', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 500 });
    mockGetPendingSyncRetryCount.mockResolvedValue(5); // MAX_RETRY_ATTEMPTS = 5

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('n1');
    expect(mockClearPendingSync).toHaveBeenCalledWith('n1');
  });

  it('handles mixed 4xx and 5xx errors correctly', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
      { note_id: 'n3' },
    ]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    // 5xx entries are retried 3 times each (FLUSH_RETRIES=2 → loop 0..2)
    mockFetch
      .mockResolvedValueOnce({ ok: false, status: 404 }) // n1 → clear (no retry)
      .mockResolvedValueOnce({ ok: false, status: 500 }) // n2 attempt 0
      .mockResolvedValueOnce({ ok: false, status: 500 }) // n2 attempt 1
      .mockResolvedValueOnce({ ok: false, status: 500 }) // n2 attempt 2
      .mockResolvedValueOnce({ ok: false, status: 503 }) // n3 attempt 0
      .mockResolvedValueOnce({ ok: false, status: 503 }) // n3 attempt 1
      .mockResolvedValueOnce({ ok: false, status: 503 }); // n3 attempt 2
    mockGetPendingSyncRetryCount
      .mockResolvedValueOnce(1) // n2 after last attempt
      .mockResolvedValueOnce(1); // n3 after last attempt

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 3 });
    expect(mockClearPendingSync).toHaveBeenCalledTimes(1); // Only n1 cleared (4xx)
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledTimes(2); // n2 and n3
  });

  it('counts 429 rate limit as failed without clearing entry', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 429 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockClearPendingSync).not.toHaveBeenCalled();
  });

  // #2126: 429 must stop the entire flush, not continue to next pending entry
  it('stops flush on first 429 instead of sending remaining requests', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
      { note_id: 'n3' },
      { note_id: 'n4' },
      { note_id: 'n5' },
    ]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 429 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    // Only the first entry should have been attempted
    expect(mockFetch).toHaveBeenCalledTimes(1);
    // No entries should be cleared (they remain for next flush)
    expect(mockClearPendingSync).not.toHaveBeenCalled();
  });
});
