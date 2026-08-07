/**
 * Regression tests for #3871: 同一秒内的服务端新版本被永久跳过（同秒边界 staleness）。
 *
 * Bug: #3866 把 skip 判定改成整秒比较
 *   `Math.floor(serverTs / 1000) <= localNote.updated_at`
 * 但本地 SQLite 存的是向下取整的整秒，服务端是亚秒精度。若设备 B 在导入
 * 后的同一秒内（更晚的亚秒时刻）更新了 note，floor 后与本地相同 → 永远
 * SKIP → 本地永久 staleness。
 *
 * Fix: 导入时额外记录服务端精确 ms（server_updated_ms 列），有该值的行为
 * 用完整精度比较（localServerMs >= serverTs → skip）；无该值的 legacy/
 * 本地编辑行回退到保守的严格整秒比较（同秒一律 fetch）。
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

// Same-second boundary: 19:00:37.100 (imported) vs 19:00:37.900 (newer server update).
const IMPORTED_MS = Date.parse('2026-08-07T19:00:37.100Z');
const NEWER_SAME_SECOND_MS = Date.parse('2026-08-07T19:00:37.900Z');
const OLDER_SECOND_MS = Date.parse('2026-08-07T19:00:36.500Z');
const FLOOR_37 = Math.floor(IMPORTED_MS / 1000);

describe('same-second server updates must not be skipped (#3871)', () => {
  it('fetches a newer update landing later within the same second (imported row)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Device A imported the 37.100 version; local floor is 37, exact ms recorded.
    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: FLOOR_37, server_updated_ms: IMPORTED_MS },
    ]);

    // Device B updated the same note at 37.900 — same second, but newer.
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:37.900Z' }],
        total: 1,
      }),
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        meta: { id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:37.900Z' },
        body: 'newer same-second content',
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(0);
    expect(result.updated).toBe(1);
    expect(mockUpdateNote).toHaveBeenCalledTimes(1);
    // The re-import records the new exact ms so the next sync skips cleanly.
    expect(mockUpdateNote).toHaveBeenCalledWith('note-1', 'Note', 'newer same-second content', {
      skipQueue: true,
      is_template: 0,
      folder: '',
      updated_at: FLOOR_37,
      server_updated_ms: NEWER_SAME_SECOND_MS,
    });
  });

  it('still skips when the server version equals the imported version (full precision)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: FLOOR_37, server_updated_ms: IMPORTED_MS },
    ]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:37.100Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    expect(result.updated).toBe(0);
    expect(mockUpdateNote).not.toHaveBeenCalled();
    expect(mockFetch).toHaveBeenCalledTimes(1); // list only, no detail fetch
  });

  it('legacy rows (no server_updated_ms) conservatively fetch on same-second server updates', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    // Row imported before the #3871 migration: only floored seconds available.
    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: FLOOR_37 },
    ]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:37.900Z' }],
        total: 1,
      }),
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        meta: { id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:37.900Z' },
        body: 'content',
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(0);
    expect(result.updated).toBe(1);
  });

  it('legacy rows still skip when the server is a full second older', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://localhost:3000');

    mockGetNoteTimestamps.mockResolvedValue([
      { id: 'note-1', updated_at: FLOOR_37 },
    ]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Note', updated_at: '2026-08-07T19:00:36.500Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    expect(mockUpdateNote).not.toHaveBeenCalled();
  });
});
