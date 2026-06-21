/**
 * Regression tests for sync.ts — vault note sync (#1242).
 *
 * Tests: getServerConfig, setServerConfig, pingBackend,
 *        syncNotesFromServer, getLastSyncTime.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  getServerConfig,
  setServerConfig,
  pingBackend,
  syncNotesFromServer,
  getLastSyncTime,
} from '../../services/sync';

// Mock db module
jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
}));

const mockCreateNote = require('../../db').createNote as jest.MockedFunction<any>;
const mockUpdateNote = require('../../db').updateNote as jest.MockedFunction<any>;
const mockGetNotes = require('../../db').getNotes as jest.MockedFunction<any>;

// Mock global fetch
const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  await AsyncStorage.clear();
  mockFetch.mockReset();
  mockGetNotes.mockResolvedValue([]);
});

// ── getServerConfig ─────────────────────────────────────────

describe('getServerConfig', () => {
  it('returns empty strings when storage is empty', async () => {
    const config = await getServerConfig();
    expect(config.url).toBe('');
    expect(config.token).toBe('');
  });

  it('reads url and token from AsyncStorage', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://192.168.1.100:3000')
      .mockResolvedValueOnce('my-token');
    const config = await getServerConfig();
    expect(config.url).toBe('http://192.168.1.100:3000');
    expect(config.token).toBe('my-token');
  });
});

// ── setServerConfig ─────────────────────────────────────────

describe('setServerConfig', () => {
  it('saves url and token to AsyncStorage', async () => {
    await setServerConfig('http://localhost:3000', 'tok123');
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('cfg_backend_url', 'http://localhost:3000');
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('cfg_backend_token', 'tok123');
  });

  it('strips trailing slashes from url', async () => {
    await setServerConfig('http://localhost:3000///', 'tok');
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('cfg_backend_url', 'http://localhost:3000');
  });

  it('removes token key when token is empty', async () => {
    await setServerConfig('http://localhost:3000', '');
    expect(AsyncStorage.removeItem).toHaveBeenCalledWith('cfg_backend_token');
  });
});

// ── pingBackend ─────────────────────────────────────────────

describe('pingBackend', () => {
  it('returns false when no url configured', async () => {
    const result = await pingBackend();
    expect(result).toBe(false);
  });

  it('returns true when health endpoint responds ok', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    const result = await pingBackend();
    expect(result).toBe(true);
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:3000/health',
      expect.objectContaining({ signal: expect.anything() }),
    );
  });

  it('returns false when health endpoint returns error', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });
    const result = await pingBackend();
    expect(result).toBe(false);
  });

  it('returns false on network error', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockFetch.mockRejectedValueOnce(new Error('Network error'));
    const result = await pingBackend();
    expect(result).toBe(false);
  });
});

// ── syncNotesFromServer ─────────────────────────────────────

describe('syncNotesFromServer', () => {
  it('throws when no backend url configured', async () => {
    await expect(syncNotesFromServer()).rejects.toThrow('未配置后端服务器地址');
  });

  it('imports new notes from server', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockGetNotes.mockResolvedValue([]);

    // First call: list notes
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Test Note', updated_at: '2026-01-01T00:00:00Z' }],
        total: 1,
      }),
    });
    // Second call: get note detail
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        meta: { id: 'note-1', title: 'Test Note' },
        body: 'Note content here',
      }),
    });

    mockCreateNote.mockResolvedValue('new-local-id');

    const result = await syncNotesFromServer();
    expect(result.imported).toBe(1);
    expect(result.updated).toBe(0);
    expect(result.skipped).toBe(0);
    expect(result.errors).toBe(0);
    expect(mockCreateNote).toHaveBeenCalledWith('Test Note');
    expect(mockUpdateNote).toHaveBeenCalledWith('new-local-id', 'Test Note', 'Note content here');
  });

  it('skips notes where local is newer', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    mockGetNotes.mockResolvedValue([
      { id: 'note-1', title: 'Old', content: '', starred: 0, folder: '', created_at: 0, updated_at: Date.now() },
    ]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Old', updated_at: '2020-01-01T00:00:00Z' }],
        total: 1,
      }),
    });

    const result = await syncNotesFromServer();
    expect(result.skipped).toBe(1);
    expect(result.imported).toBe(0);
    expect(result.updated).toBe(0);
  });

  it('updates existing notes when server is newer', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    mockGetNotes.mockResolvedValue([
      { id: 'note-1', title: 'Old', content: 'old content', starred: 0, folder: '', created_at: 0, updated_at: 1000 },
    ]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Updated', updated_at: '2026-06-01T00:00:00Z' }],
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
    expect(result.imported).toBe(0);
    expect(mockUpdateNote).toHaveBeenCalledWith('note-1', 'Updated', 'new content');
  });

  it('counts errors when note detail fetch fails', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockGetNotes.mockResolvedValue([]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({
        notes: [{ id: 'note-1', title: 'Fail', updated_at: '2026-06-01T00:00:00Z' }],
        total: 1,
      }),
    });
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });

    const result = await syncNotesFromServer();
    expect(result.errors).toBe(1);
    expect(result.imported).toBe(0);
  });

  it('throws when list endpoint fails', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    mockFetch.mockResolvedValueOnce({ ok: false, status: 403 });

    await expect(syncNotesFromServer()).rejects.toThrow('获取笔记列表失败: 403');
  });

  it('uses Authorization header when token is set', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('my-secret-token');
    mockGetNotes.mockResolvedValue([]);

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ notes: [], total: 0 }),
    });

    await syncNotesFromServer();
    expect(mockFetch).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: 'Bearer my-secret-token',
        }),
      }),
    );
  });
});

// ── getLastSyncTime ─────────────────────────────────────────

describe('getLastSyncTime', () => {
  it('returns null when no sync has happened', async () => {
    const result = await getLastSyncTime();
    expect(result).toBeNull();
  });

  it('returns the stored sync time', async () => {
    (AsyncStorage.getItem as jest.Mock).mockResolvedValueOnce('2026-06-22T00:00:00Z');
    const result = await getLastSyncTime();
    expect(result).toBe('2026-06-22T00:00:00Z');
  });
});
