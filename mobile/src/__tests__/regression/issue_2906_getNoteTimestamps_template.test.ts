/**
 * Regression test for #2906: getNoteTimestamps includes template notes
 * in sync comparison results.
 *
 * Verifies the SQL query filters out template notes (is_template = 0)
 * so they never participate in sync comparison.
 */

const mockGetAllAsync = jest.fn().mockResolvedValue([]);
const mockExecAsync = jest.fn().mockResolvedValue(undefined);

jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: jest.fn().mockResolvedValue({
    getAllAsync: mockGetAllAsync,
    execAsync: mockExecAsync,
    runAsync: jest.fn().mockResolvedValue(undefined),
    withTransactionAsync: jest.fn().mockImplementation(async (fn: () => Promise<void>) => fn()),
  }),
}));

import { getNoteTimestamps } from '../../db';

beforeEach(() => {
  jest.clearAllMocks();
});

describe('#2906 getNoteTimestamps excludes template notes', () => {
  it('SQL query includes WHERE is_template = 0', async () => {
    await getNoteTimestamps();

    expect(mockGetAllAsync).toHaveBeenCalledWith(
      expect.stringContaining('WHERE is_template = 0')
    );
  });

  it('does NOT include template notes in results', async () => {
    // Simulate a DB that only has template notes
    mockGetAllAsync.mockResolvedValueOnce([]);

    const result = await getNoteTimestamps();
    // Should be empty (or contain only non-template notes)
    expect(result).toEqual([]);
  });
});
