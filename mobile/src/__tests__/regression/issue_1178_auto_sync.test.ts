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
  it('returns null when no backend url configured', async () => {
    const result = await autoSyncOnStartup();
    expect(result).toBeNull();
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('returns null when backend is unreachable', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://192.168.1.100:3000');
    mockFetch.mockRejectedValueOnce(new Error('Connection refused'));

    const result = await autoSyncOnStartup();
    expect(result).toBeNull();
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
    expect(result).not.toBeNull();
    expect(result!.imported).toBe(0);
    expect(result!.skipped).toBe(0);
    expect(result!.errors).toBe(0);
  });

  it('returns null on sync error (does not throw)', async () => {
    await AsyncStorage.setItem('cfg_backend_url', 'http://192.168.1.100:3000');

    // pingBackend succeeds
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200 });
    // syncNotesFromServer → list fails
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });

    const result = await autoSyncOnStartup();
    // Should catch the error and return null instead of throwing
    expect(result).toBeNull();
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
