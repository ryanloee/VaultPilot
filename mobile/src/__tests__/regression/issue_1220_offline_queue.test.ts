/**
 * Regression tests for offline edit queue (#1220).
 *
 * Verifies:
 * 1. queuePendingSync calls INSERT OR REPLACE
 * 2. getPendingSyncCount returns correct count
 * 3. clearPendingSync deletes by note_id
 * 4. clearAllPendingSyncs deletes all
 */

// Mock expo-sqlite inline to avoid circular dependency
const mockRunAsync = jest.fn().mockResolvedValue(undefined);
const mockGetFirstAsync = jest.fn().mockResolvedValue(null);
const mockGetAllAsync = jest.fn().mockResolvedValue([]);
const mockExecAsync = jest.fn().mockResolvedValue(undefined);

jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: jest.fn().mockResolvedValue({
    execAsync: mockExecAsync,
    getAllAsync: mockGetAllAsync,
    getFirstAsync: mockGetFirstAsync,
    runAsync: mockRunAsync,
    withTransactionAsync: jest.fn().mockImplementation(async (fn: () => Promise<void>) => fn()),
  }),
}));

import {
  queuePendingSync,
  getPendingSyncCount,
  getPendingSyncs,
  clearPendingSync,
  clearAllPendingSyncs,
} from '../../db';

beforeEach(() => {
  jest.clearAllMocks();
});

describe('Offline Sync Queue (#1220)', () => {
  it('queuePendingSync should UPSERT with note_id and action', async () => {
    await queuePendingSync('note-1', 'update');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('INSERT INTO pending_syncs'),
      ['note-1', 'update']
    );
  });

  it('queuePendingSync should default action to update', async () => {
    await queuePendingSync('note-2');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('INSERT INTO pending_syncs'),
      ['note-2', 'update']
    );
  });

  it('getPendingSyncCount should return count from DB', async () => {
    mockGetFirstAsync.mockResolvedValueOnce({ c: 5 });
    const count = await getPendingSyncCount();
    expect(count).toBe(5);
  });

  it('getPendingSyncCount should return 0 when no rows', async () => {
    mockGetFirstAsync.mockResolvedValueOnce(null);
    const count = await getPendingSyncCount();
    expect(count).toBe(0);
  });

  it('getPendingSyncs should return all entries ordered by created_at', async () => {
    const mockRows = [
      { id: 1, note_id: 'a', action: 'update' },
      { id: 2, note_id: 'b', action: 'create' },
    ];
    mockGetAllAsync.mockResolvedValueOnce(mockRows);
    const result = await getPendingSyncs();
    expect(result).toEqual(mockRows);
  });

  it('clearPendingSync should DELETE by note_id', async () => {
    await clearPendingSync('note-1');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('DELETE FROM pending_syncs WHERE note_id = ?'),
      ['note-1']
    );
  });

  it('clearAllPendingSyncs should DELETE all rows', async () => {
    await clearAllPendingSyncs();
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('DELETE FROM pending_syncs')
    );
  });
});
