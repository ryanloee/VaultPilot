/**
 * Regression test for #2904: addTag/removeTag not wrapped in transaction.
 *
 * Verifies both functions call withTransactionAsync so that the two
 * DB writes are atomic — if the second UPDATE fails the first INSERT
 * is rolled back.
 */

// Mock expo-sqlite inline — must include execAsync for initDb to work
const mockRunAsync = jest.fn().mockResolvedValue(undefined);
const mockExecAsync = jest.fn().mockResolvedValue(undefined);
const mockWithTransactionAsync = jest
  .fn()
  .mockImplementation(async (fn: () => Promise<void>) => fn());
const mockGetAllAsync = jest.fn().mockResolvedValue([]);

jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: jest.fn().mockResolvedValue({
    runAsync: mockRunAsync,
    execAsync: mockExecAsync,
    withTransactionAsync: mockWithTransactionAsync,
    getAllAsync: mockGetAllAsync,
  }),
}));

import { addTag, removeTag } from '../../db';

beforeEach(() => {
  jest.clearAllMocks();
  mockWithTransactionAsync.mockImplementation(async (fn: () => Promise<void>) => fn());
});

describe('#2904 addTag wraps writes in transaction', () => {
  it('calls withTransactionAsync (wraps writes atomically)', async () => {
    await addTag('note-1', 'urgent');

    // withTransactionAsync must have been called — this is the core assertion
    expect(mockWithTransactionAsync).toHaveBeenCalled();
  });

  it('still works with skipQueue', async () => {
    await addTag('note-1', 'urgent', { skipQueue: true });
    expect(mockWithTransactionAsync).toHaveBeenCalled();
  });
});

describe('#2904 removeTag wraps writes in transaction', () => {
  it('calls withTransactionAsync (wraps writes atomically)', async () => {
    await removeTag('note-1', 'urgent');
    expect(mockWithTransactionAsync).toHaveBeenCalled();
  });

  it('still works with skipQueue', async () => {
    await removeTag('note-1', 'urgent', { skipQueue: true });
    expect(mockWithTransactionAsync).toHaveBeenCalled();
  });
});
