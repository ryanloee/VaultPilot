/**
 * Regression test for #1447: pending_syncs INSERT OR REPLACE deduplication.
 *
 * Before fix: INSERT OR REPLACE with autoincrement PK always inserted new rows
 * because the replacement was on `id` (auto-generated), not `note_id`.
 * After fix: UNIQUE index on note_id ensures INSERT OR REPLACE deduplicates correctly.
 */

// Mock expo-sqlite inline
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
  clearPendingSync,
} from '../../db';

describe('issue #1447 — pending_syncs deduplication', () => {
  // These tests run in order; getDb() is a singleton so migration runs once.
  // First test triggers getDb() and can verify migration calls.

  it('migration should create UNIQUE index on pending_syncs.note_id', async () => {
    // This is the first call — triggers getDb() and all migrations
    mockGetFirstAsync.mockResolvedValueOnce({ c: 0 });
    await getPendingSyncCount();

    // Verify migration checked sqlite_master for the index
    const getAllCalls = mockGetAllAsync.mock.calls.map((c: any[]) => c[0] as string);
    const checksIndex = getAllCalls.some((sql: string) =>
      typeof sql === 'string' && sql.includes('sqlite_master') && sql.includes('idx_pending_syncs_note_id')
    );
    expect(checksIndex).toBe(true);

    // Verify migration created the UNIQUE index
    const execCalls = mockExecAsync.mock.calls.map((c: any[]) => c[0] as string);
    const createsIndex = execCalls.some((sql: string) =>
      typeof sql === 'string' && sql.includes('CREATE UNIQUE INDEX') && sql.includes('idx_pending_syncs_note_id')
    );
    expect(createsIndex).toBe(true);
  });

  it('queuePendingSync should use UPSERT (works with UNIQUE index)', async () => {
    jest.clearAllMocks();
    await queuePendingSync('note-abc', 'update');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('INSERT INTO pending_syncs'),
      ['note-abc', 'update']
    );
  });

  it('queuePendingSync default action should be update', async () => {
    jest.clearAllMocks();
    await queuePendingSync('note-xyz');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('INSERT INTO pending_syncs'),
      ['note-xyz', 'update']
    );
  });

  it('clearPendingSync should delete by note_id', async () => {
    jest.clearAllMocks();
    await clearPendingSync('note-abc');
    expect(mockRunAsync).toHaveBeenCalledWith(
      expect.stringContaining('DELETE FROM pending_syncs WHERE note_id = ?'),
      ['note-abc']
    );
  });
});
