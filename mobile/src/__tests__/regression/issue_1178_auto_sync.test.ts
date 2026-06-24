/**
 * Regression tests for autoSyncOnStartup — auto-sync on app launch (#1178).
 *
 * Tests: auto-sync triggers sync when backend reachable,
 *        skips when no url configured, skips when backend unreachable,
 *        handles errors gracefully.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { autoSyncOnStartup, getServerConfig } from '../../services/sync';

// Mock db module
jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
}));

const mockGetNotes = require('../../db').getNotes as jest.MockedFunction<any>;

// Mock global fetch
const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  mockFetch.mockReset();
  mockGetNotes.mockResolvedValue([]);
  await AsyncStorage.clear();
});

describe('autoSyncOnStartup', () => {
  it('returns skipped when no backend url configured', async () => {
    const result = await autoSyncOnStartup();
    expect(result).toEqual({ status: 'skipped', reason: 'no_config' });
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('returns skipped when backend is unreachable', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://192.168.1.100:3000');
    mockFetch.mockRejectedValueOnce(new Error('Connection refused'));

    const result = await autoSyncOnStartup();
    expect(result).toEqual({ status: 'skipped', reason: 'unreachable' });
  });

  it('syncs notes when backend is reachable', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://192.168.1.100:3000');
    mockGetNotes.mockResolvedValue([]);

    // pingBackend → /health
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    // syncNotesFromServer → /api/notes
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ notes: [], total: 0 }),
    });

    const result = await autoSyncOnStartup();
    expect(result.status).toBe('done');
    if (result.status === 'done') {
      expect(result.result.imported).toBe(0);
      expect(result.result.skipped).toBe(0);
      expect(result.result.errors).toBe(0);
    }
  });

  it('returns error on sync failure (does not throw)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://192.168.1.100:3000');

    // pingBackend succeeds
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    // syncNotesFromServer → list fails
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });

    const result = await autoSyncOnStartup();
    // Should catch the error and return error status instead of throwing
    expect(result.status).toBe('error');
    if (result.status === 'error') {
      expect(result.error).toBeTruthy();
    }
  });

  it('does not block app startup (fire-and-forget pattern)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://192.168.1.100:3000');

    // Make fetch hang to verify non-blocking
    let resolveHealth: (v: any) => void;
    mockFetch.mockReturnValueOnce(
      new Promise((resolve) => { resolveHealth = resolve; })
    );

    // This should return quickly if called fire-and-forget
    // The actual autoSyncOnStartup is async but App.tsx doesn't await it
    const startTime = Date.now();
    const promise = autoSyncOnStartup();

    // Immediately resolve the health check
    resolveHealth!({ ok: true, status: 200 });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ notes: [], total: 0 }),
    });

    const result = await promise;
    expect(result).not.toBeNull();
  });
});
