/**
 * Regression test for sync timestamp unit mismatch (#1390).
 *
 * Bug: syncNotesFromServer compared localNote.updated_at (seconds from
 * SQLite strftime('%s')) with serverTs (milliseconds from Date.getTime()).
 * Since seconds << milliseconds, the comparison always failed, causing
 * every sync to re-download all notes even when local was newer.
 *
 * Fix: multiply localNote.updated_at by 1000 before comparing.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer } from '../../services/sync';

jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
  getNoteTimestamps: jest.fn(),
}));

const mockCreateNote = require('../../db').createNote as jest.MockedFunction<any>;
const mockUpdateNote = require('../../db').updateNote as jest.MockedFunction<any>;
const mockGetNotes = require('../../db').getNotes as jest.MockedFunction<any>;
const mockGetNoteTimestamps = require('../../db').getNoteTimestamps as jest.MockedFunction<any>;

const mockFetch = jest.fn();
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  await AsyncStorage.clear();
  mockFetch.mockReset();
  mockGetNoteTimestamps.mockResolvedValue([]);
});

describe('sync timestamp unit mismatch (#1390)', () => {
  it('skips notes when local updated_at (seconds) is newer than server timestamp', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Local note updated at 2026-06-23T12:00:00Z in seconds (SQLite format)
    const localSeconds = Math.floor(new Date('2026-06-23T12:00:00Z').getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: localSeconds },
    ]);

    // Server note updated at 2026-06-23T10:00:00Z (older than local)
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Server', updated_at: '2026-06-23T10:00:00Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    expect(result.updated).toBe(0);
    expect(result.imported).toBe(0);
    expect(mockUpdateNote).not.toHaveBeenCalled();
    expect(mockFetch).toHaveBeenCalledTimes(1); // Only list call, no detail fetch
  });

  it('updates notes when server timestamp is newer than local (seconds)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Local note updated at 2026-06-23T10:00:00Z in seconds
    const localSeconds = Math.floor(new Date('2026-06-23T10:00:00Z').getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: localSeconds },
    ]);

    // Server note updated at 2026-06-23T12:00:00Z (newer than local)
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Updated', updated_at: '2026-06-23T12:00:00Z' }],
        total: 1,
      }),
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        meta: { id: 'note-1', title: 'Updated' },
        body: 'new content',
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.updated).toBe(1);
    expect(result.skipped).toBe(0);
    expect(mockUpdateNote).toHaveBeenCalledWith('note-1', 'Updated', 'new content', { skipQueue: true, is_template: 0, folder: '', updated_at: undefined });
  });

  it('skips when local and server timestamps are equal (boundary)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Both at exactly 2026-06-23T12:00:00Z
    const ts = new Date('2026-06-23T12:00:00Z');
    const localSeconds = Math.floor(ts.getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: localSeconds },
    ]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-06-23T12:00:00Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    // localSeconds * 1000 === serverTs, so >= is true → skipped
    expect(result.skipped).toBe(1);
    expect(result.updated).toBe(0);
  });
});
