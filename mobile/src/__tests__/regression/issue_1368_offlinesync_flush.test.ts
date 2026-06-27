/**
 * Regression tests for offlineSync flushPendingSyncs (#1368, #1596).
 *
 * Verifies:
 * 1. Empty server config → returns {synced: 0, failed: 0}
 * 2. Empty pending queue → returns {synced: 0, failed: 0}
 * 3. All syncs succeed → correct counts
 * 4. Deleted note locally → clears pending, counts as synced
 * 5. Server returns non-ok → increments failed
 * 6. Network error mid-flush → stops and returns partial counts
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
const mockIncrementPendingSyncRetry = jest.fn();
const mockGetPendingSyncRetryCount = jest.fn();

jest.mock('../../services/sync', () => ({
  getServerConfig: mockGetServerConfig,
}));

jest.mock('../../db', () => ({
  getPendingSyncs: mockGetPendingSyncs,
  getNote: mockGetNote,
  clearPendingSync: mockClearPendingSync,
  getPendingSyncCount: mockGetPendingSyncCount,
  queuePendingSync: mockQueuePendingSync,
  incrementPendingSyncRetry: mockIncrementPendingSyncRetry,
  getPendingSyncRetryCount: mockGetPendingSyncRetryCount,
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

// Mock AbortSignal.timeout
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).AbortSignal = { timeout: jest.fn().mockReturnValue(undefined) };

import { flushPendingSyncs } from '../../utils/offlineSync';

describe('flushPendingSyncs', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetServerConfig.mockResolvedValue({ url: 'http://localhost:8080', token: 'tok123' });
    mockGetPendingSyncs.mockResolvedValue([]);
    mockGetNote.mockResolvedValue(null);
    mockClearPendingSync.mockResolvedValue(undefined);
    mockIncrementPendingSyncRetry.mockResolvedValue(undefined);
    mockGetPendingSyncRetryCount.mockResolvedValue(0);
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

  it('counts server error as failed', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 500 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('n1');
  });

  it('stops flushing on network error and returns partial counts', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
      { note_id: 'n3' },
    ]);
    // First note succeeds, second throws network error
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch
      .mockResolvedValueOnce({ ok: true })
      .mockRejectedValueOnce(new Error('Network request failed'));

    const result = await flushPendingSyncs();
    expect(result.synced).toBe(1);
    expect(result.failed).toBe(1);
    // Should have stopped after error — n3 never processed
    expect(mockFetch).toHaveBeenCalledTimes(2);
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
    // First returns 500, second succeeds, third succeeds
    mockFetch
      .mockResolvedValueOnce({ ok: false, status: 500 })
      .mockResolvedValueOnce({ ok: true })
      .mockResolvedValueOnce({ ok: true });

    const result = await flushPendingSyncs();
    // n1 failed (server error), n2 and n3 synced
    expect(result.synced).toBe(2);
    expect(result.failed).toBe(1);
    expect(mockFetch).toHaveBeenCalledTimes(3);
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

  it('passes AbortSignal.timeout to fetch', async () => {
    const mockTimeout = (globalThis as any).AbortSignal.timeout as jest.Mock;
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'n1' }]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: true });

    await flushPendingSyncs();
    expect(mockTimeout).toHaveBeenCalledWith(10000);
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
    expect(mockGetPendingSyncRetryCount).toHaveBeenCalledWith('n1');
    expect(mockClearPendingSync).toHaveBeenCalledWith('n1');
  });

  it('handles mixed 4xx and 5xx errors correctly', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'n1' },
      { note_id: 'n2' },
      { note_id: 'n3' },
    ]);
    mockGetNote.mockResolvedValue({ id: 'n1', title: 'T', content: 'C' });
    mockFetch
      .mockResolvedValueOnce({ ok: false, status: 404 }) // 4xx: clear
      .mockResolvedValueOnce({ ok: false, status: 500 }) // 5xx: retry
      .mockResolvedValueOnce({ ok: false, status: 503 }); // 5xx: retry
    mockGetPendingSyncRetryCount
      .mockResolvedValueOnce(1) // n2 after first retry
      .mockResolvedValueOnce(1); // n3 after first retry

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
