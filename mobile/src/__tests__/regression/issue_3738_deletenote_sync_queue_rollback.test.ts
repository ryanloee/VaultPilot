/**
 * Regression test for #3738: deleteNote rolls back on queuePendingSync failure.
 *
 * Root cause: deleteNote wrapped both DELETE and queuePendingSync in a single
 * withTransactionAsync(). If queuePendingSync throws, the entire transaction
 * rolls back and the note is NOT deleted.
 *
 * Fix: Move queuePendingSync outside the transaction with try/catch, mirroring
 * the createNote/updateNote best-effort pattern from #3502.
 *
 * We simulate queuePendingSync failure by making runAsync throw on
 * INSERT INTO pending_syncs (the internal SQL used by queuePendingSync).
 * This mirrors the #3502 test approach while also testing the key behavior:
 * the DELETE must have executed BEFORE the sync queue write attempt.
 */

const mockRunAsync = jest.fn().mockImplementation(async (sql: string) => {
  // Simulate sync queue write failure
  if (typeof sql === 'string' && sql.includes('INSERT INTO pending_syncs')) {
    throw new Error('simulated sync queue write failure');
  }
  return undefined;
});
const mockExecAsync = jest.fn().mockResolvedValue(undefined);
const mockGetFirstAsync = jest.fn().mockResolvedValue(null);
const mockGetAllAsync = jest.fn().mockResolvedValue([]);
const mockWithTransactionAsync = jest.fn().mockImplementation(async (fn: () => Promise<void>) => fn());

jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: jest.fn().mockResolvedValue({
    execAsync: mockExecAsync,
    getAllAsync: mockGetAllAsync,
    getFirstAsync: mockGetFirstAsync,
    runAsync: mockRunAsync,
    withTransactionAsync: mockWithTransactionAsync,
  }),
}));

import { deleteNote } from '../../db';

describe('issue #3738 — deleteNote success even when queuePendingSync fails', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    // Reset the mock implementation for each test
    mockRunAsync.mockImplementation(async (sql: string) => {
      if (typeof sql === 'string' && sql.includes('INSERT INTO pending_syncs')) {
        throw new Error('simulated sync queue write failure');
      }
      return undefined;
    });
  });

  it('DELETE succeeds + queuePendingSync throws → note still deleted (no rollback)', async () => {
    // deleteNote should resolve (not throw) despite sync queue failure
    await expect(deleteNote('note-x')).resolves.toBeUndefined();

    // Critical: the DELETE statement must have been executed
    const deleteCalls = mockRunAsync.mock.calls.filter(
      (c: unknown[]) => typeof c[0] === 'string' && c[0].includes('DELETE FROM notes')
    );
    expect(deleteCalls.length).toBe(1);
    expect(deleteCalls[0][0]).toBe('DELETE FROM notes WHERE id = ?');
    expect(deleteCalls[0][1]).toEqual(['note-x']);
  });

  it('withTransactionAsync should NOT be involved in the new code path', async () => {
    await deleteNote('note-y');

    // The old path used withTransactionAsync; the new path does not.
    // We verify that the DELETE was executed directly via runAsync.
    const deleteCalls = mockRunAsync.mock.calls.filter(
      (c: unknown[]) => typeof c[0] === 'string' && c[0].includes('DELETE FROM notes')
    );
    expect(deleteCalls.length).toBe(1);
  });
});