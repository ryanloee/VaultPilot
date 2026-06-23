/**
 * Regression tests for offlineSync flushPendingSyncs (#1368).
 *
 * Verifies:
 * 1. Empty server config → returns {synced: 0, failed: 0}
 * 2. Empty pending queue → returns {synced: 0, failed: 0}
 * 3. All syncs succeed → correct counts
 * 4. Deleted note locally → clears pending, counts as synced
 * 5. Server returns non-ok → increments failed
 * 6. Network error mid-flush → stops and returns partial counts
 */

const mockGetServerConfig = jest.fn();
const mockGetPendingSyncs = jest.fn();
const mockGetNote = jest.fn();
const mockClearPendingSync = jest.fn();
const mockGetPendingSyncCount = jest.fn().mockResolvedValue(0);
const mockQueuePendingSync = jest.fn();

jest.mock('../../services/sync', () => ({
  getServerConfig: mockGetServerConfig,
}));

jest.mock('../../db', () => ({
  getPendingSyncs: mockGetPendingSyncs,
  getNote: mockGetNote,
  clearPendingSync: mockClearPendingSync,
  getPendingSyncCount: mockGetPendingSyncCount,
  queuePendingSync: mockQueuePendingSync,
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
    expect(mockClearPendingSync).not.toHaveBeenCalled();
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
});
