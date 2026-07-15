/**
 * Regression test for #2933: offlineSync DELETE 4xx client error handling.
 *
 * Before fix: 4xx on DELETE immediately cleared pending sync entry,
 * permanently abandoning the delete (orphaning the note on server).
 *
 * After fix: 4xx on DELETE keeps the pending entry and relies on
 * cross-cycle retry mechanism (incrementPendingSyncRetry). Only clears
 * after MAX_RETRY_ATTEMPTS are exhausted.
 */

const mockGetServerConfig = jest.fn();
const mockGetPendingSyncs = jest.fn();
const mockGetNote = jest.fn();
const mockClearPendingSync = jest.fn();
const mockDeleteNote = jest.fn();
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
  deleteNote: mockDeleteNote,
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

const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

import { flushPendingSyncs } from '../../utils/offlineSync';

describe('flushPendingSyncs — DELETE 4xx regression (#2933)', () => {
  jest.setTimeout(30000);
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetServerConfig.mockResolvedValue({ url: 'http://localhost:8080', token: 'tok123' });
    mockGetPendingSyncs.mockResolvedValue([]);
    mockGetNote.mockResolvedValue(null);
    mockClearPendingSync.mockResolvedValue(undefined);
    mockIncrementPendingSyncRetry.mockResolvedValue(undefined);
    mockGetPendingSyncRetryCount.mockResolvedValue(0);
    mockGetNoteTags.mockResolvedValue([]);
  });

  it('DELETE 403: keeps pending entry, increments retry, does NOT clear', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-note-1' }]);
    // getPendingSync returns current action as 'delete'
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 1, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-note-1', title: 'DeletedNote', content: 'Old Content' });
    mockFetch.mockResolvedValue({ ok: false, status: 403 });
    mockGetPendingSyncRetryCount.mockResolvedValue(1); // below MAX (5)

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    // Must NOT clear — keep entry for retry on next flush cycle
    expect(mockClearPendingSync).not.toHaveBeenCalled();
    // Must increment retry count
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('del-note-1');
  });

  it('DELETE 400: keeps pending entry on bad request', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-note-2' }]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 2, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-note-2', title: 'N2', content: 'C2' });
    mockFetch.mockResolvedValue({ ok: false, status: 400 });
    mockGetPendingSyncRetryCount.mockResolvedValue(2);

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockClearPendingSync).not.toHaveBeenCalled();
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('del-note-2');
  });

  it('DELETE 401: keeps pending entry, does NOT clear', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-note-3' }]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 3, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-note-3', title: 'N3', content: 'C3' });
    mockFetch.mockResolvedValue({ ok: false, status: 401 });
    mockGetPendingSyncRetryCount.mockResolvedValue(3);

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockClearPendingSync).not.toHaveBeenCalled();
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('del-note-3');
  });

  it('DELETE 4xx: clears entry only after MAX_RETRY_ATTEMPTS exhausted', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-maxed' }]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 4, note_id: noteId, action: 'delete', retry_count: 5 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-maxed', title: 'Old', content: 'Old' });
    mockFetch.mockResolvedValue({ ok: false, status: 403 });
    mockGetPendingSyncRetryCount.mockResolvedValue(5); // MAX_RETRY_ATTEMPTS = 5

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    // After MAX retries, clear the entry
    expect(mockClearPendingSync).toHaveBeenCalledWith('del-maxed');
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalledWith('del-maxed');
  });

  it('DELETE 404: still clears and counts as synced (existing behavior unchanged)', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-404' }]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 5, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-404', title: 'Already', content: 'Deleted' });
    mockFetch.mockResolvedValue({ ok: false, status: 404 });

    const result = await flushPendingSyncs();
    // 404 on delete = success (already deleted)
    expect(result).toEqual({ synced: 1, failed: 0 });
    expect(mockClearPendingSync).toHaveBeenCalledWith('del-404');
    expect(mockIncrementPendingSyncRetry).not.toHaveBeenCalled();
  });

  it('DELETE 408: keeps entry with retry (existing transient behavior unchanged)', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-408' }]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 6, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-408', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 408 });
    mockGetPendingSyncRetryCount.mockResolvedValue(1);

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    expect(mockClearPendingSync).not.toHaveBeenCalled();
    expect(mockIncrementPendingSyncRetry).toHaveBeenCalled();
  });

  it('DELETE 429: stops flush, does NOT clear entry', async () => {
    mockGetPendingSyncs.mockResolvedValue([
      { note_id: 'del-429' },
      { note_id: 'next-note' },
    ]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 7, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-429', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: false, status: 429 });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 0, failed: 1 });
    // only one fetch call (stop entire flush)
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockClearPendingSync).not.toHaveBeenCalled();
  });

  it('DELETE success (200): clears entry, counts as synced (unchanged)', async () => {
    mockGetPendingSyncs.mockResolvedValue([{ note_id: 'del-ok' }]);
    mockGetPendingSync.mockImplementation((noteId: string) =>
      Promise.resolve({ id: 8, note_id: noteId, action: 'delete', retry_count: 0 })
    );
    mockGetNote.mockResolvedValue({ id: 'del-ok', title: 'T', content: 'C' });
    mockFetch.mockResolvedValue({ ok: true });

    const result = await flushPendingSyncs();
    expect(result).toEqual({ synced: 1, failed: 0 });
    expect(mockClearPendingSync).toHaveBeenCalledWith('del-ok');
  });
});