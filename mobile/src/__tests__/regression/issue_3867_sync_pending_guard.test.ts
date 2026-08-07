/**
 * Regression tests for #3867: 拉取同步不检查 pending_syncs，
 * 服务端较新时静默覆盖本地离线编辑（数据丢失竞态）。
 *
 * Bug: syncNotesFromServer classification only compared localNote.updated_at
 * vs server timestamp, ignoring pending_syncs. A note with a queued local
 * edit (offline edit → queuePendingSync) whose server version is newer got
 * pulled and overwritten via updateNote({skipQueue: true}), then the offline
 * flush pushed the (now server-clobbered) content back — silently discarding
 * the user's offline edit.
 *
 * Fix: classification excludes notes present in pending_syncs — local flush
 * pushes first, pull happens after.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer } from '../../services/sync';

jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
  getNoteTimestamps: jest.fn(),
  getPendingSyncs: jest.fn(),
  getNoteTags: jest.fn().mockResolvedValue([]),
  addTag: jest.fn(),
  removeTag: jest.fn(),
}));

const mockUpdateNote = require('../../db').updateNote as jest.MockedFunction<any>;
const mockCreateNote = require('../../db').createNote as jest.MockedFunction<any>;
const mockGetNoteTimestamps = require('../../db').getNoteTimestamps as jest.MockedFunction<any>;
const mockGetPendingSyncs = require('../../db').getPendingSyncs as jest.MockedFunction<any>;

const mockFetch = jest.fn();
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  await AsyncStorage.clear();
  mockFetch.mockReset();
  mockGetNoteTimestamps.mockResolvedValue([]);
  mockGetPendingSyncs.mockResolvedValue([]);
});

describe('sync pull respects pending_syncs (#3867)', () => {
  it('does NOT overwrite a note with a queued local edit, even when server is newer', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Local note: user edited offline at 19:00:30 (floored seconds).
    const localSeconds = Math.floor(new Date('2026-08-07T19:00:30Z').getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([{ id: 'note-1', updated_at: localSeconds }]);

    // Pending sync entry: the offline edit is queued and waiting to flush.
    mockGetPendingSyncs.mockResolvedValue([{ id: 1, note_id: 'note-1', action: 'update', retry_count: 0 }]);

    // Server version is NEWER (19:00:40) — old code would pull & clobber.
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Server version', updated_at: '2026-08-07T19:00:40.123456789Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1); // deferred — local flush wins
    expect(result.updated).toBe(0);
    // No detail fetch → no overwrite of the local offline edit.
    expect(mockUpdateNote).not.toHaveBeenCalled();
    expect(mockFetch).toHaveBeenCalledTimes(1); // list only, no detail fetch
  });

  it('still pulls normally when there is no pending entry for the note', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    const localSeconds = Math.floor(new Date('2026-08-07T19:00:30Z').getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([{ id: 'note-1', updated_at: localSeconds }]);
    // No pending entry for note-1 (empty queue).
    mockGetPendingSyncs.mockResolvedValue([]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Server', updated_at: '2026-08-07T19:00:40.000000000Z' }],
        total: 1,
      }),
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        meta: { id: 'note-1', title: 'Server', updated_at: '2026-08-07T19:00:40.000000000Z' },
        body: 'server content',
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(0);
    expect(result.updated).toBe(1);
    expect(mockUpdateNote).toHaveBeenCalledTimes(1);
  });

  it('does NOT re-create a locally-deleted note that has a pending delete entry', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Note was deleted locally → no local row; pending delete queued.
    mockGetNoteTimestamps.mockResolvedValue([]);
    mockGetPendingSyncs.mockResolvedValue([{ id: 1, note_id: 'note-1', action: 'delete', retry_count: 0 }]);

    // Server still lists the note (delete hasn't flushed yet).
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:40.000000000Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    // Old code: localMap miss → notesToFetch → createNote resurrects the note.
    expect(mockCreateNote).not.toHaveBeenCalled();
    expect(mockFetch).toHaveBeenCalledTimes(1); // list only
  });
});
