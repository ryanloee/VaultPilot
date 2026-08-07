/**
 * Regression tests for #3866: 增量同步 skip 判定因时间戳精度不匹配永远不生效。
 *
 * Bug: server `updated_at` is RFC3339 with sub-second precision (chrono
 * to_rfc3339, e.g. "2026-08-07T19:00:37.123456789Z") while local SQLite stores
 * floored whole seconds (parseServerTimestamp floors). The old comparison
 * `localNote.updated_at * 1000 >= serverTs` was almost never true, so every
 * sync re-downloaded all note bodies (skipped ≈ 0).
 *
 * Fix: compare at whole-second precision — skip when
 * `Math.floor(serverTs / 1000) <= localNote.updated_at`.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer } from '../../services/sync';

jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
  getNoteTimestamps: jest.fn(),
  getPendingSyncs: jest.fn().mockResolvedValue([]),
  getNoteTags: jest.fn().mockResolvedValue([]),
  addTag: jest.fn(),
  removeTag: jest.fn(),
}));

const mockUpdateNote = require('../../db').updateNote as jest.MockedFunction<any>;
const mockGetNoteTimestamps = require('../../db').getNoteTimestamps as jest.MockedFunction<any>;

const mockFetch = jest.fn();
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  await AsyncStorage.clear();
  mockFetch.mockReset();
  mockGetNoteTimestamps.mockResolvedValue([]);
});

describe('sync skip detection with sub-second server timestamps (#3866)', () => {
  it('skips notes when local floored-seconds equal the server sub-second timestamp', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Server timestamp with sub-second precision (chrono to_rfc3339).
    const serverTs = '2026-08-07T19:00:37.123456789Z';
    // Local stores floored whole seconds (parseServerTimestamp behavior).
    const localSeconds = Math.floor(new Date(serverTs).getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([{ id: 'note-1', updated_at: localSeconds }]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: serverTs }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    expect(result.updated).toBe(0);
    expect(result.imported).toBe(0);
    // No detail fetch → updateNote never called.
    expect(mockUpdateNote).not.toHaveBeenCalled();
    expect(mockFetch).toHaveBeenCalledTimes(1); // list only
  });

  it('still fetches when server is genuinely newer (whole-second boundary crossed)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Local note is at 19:00:37 (floored).
    const localSeconds = Math.floor(new Date('2026-08-07T19:00:37.900000000Z').getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([{ id: 'note-1', updated_at: localSeconds }]);

    // Server updated at 19:00:38 — a whole second later → must fetch.
    const serverTs = '2026-08-07T19:00:38.000000000Z';
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: serverTs }],
        total: 1,
      }),
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        meta: { id: 'note-1', title: 'Note', updated_at: serverTs },
        body: 'new content',
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(0);
    expect(result.updated).toBe(1);
    expect(mockUpdateNote).toHaveBeenCalledTimes(1);
  });

  it('skips when local is newer than server (whole-second comparison)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    const localSeconds = Math.floor(new Date('2026-08-07T19:00:37Z').getTime() / 1000);
    mockGetNoteTimestamps.mockResolvedValue([{ id: 'note-1', updated_at: localSeconds }]);

    // Server older (19:00:36) — sub-second precision on the older side too.
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:36.999999999Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    expect(mockUpdateNote).not.toHaveBeenCalled();
  });
});
