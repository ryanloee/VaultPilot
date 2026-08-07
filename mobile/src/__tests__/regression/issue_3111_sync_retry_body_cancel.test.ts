/**
 * Regression test for sync response body leak on retry (#3111).
 *
 * Bug: When sync.ts received a retryable HTTP status (429/502/503/504), it
 * called `continue` to retry without first cancelling the response body.
 * This leaked the underlying HTTP connection on every retry. With detail-fetch
 * concurrency of 5, a single sync round could leak up to 5 connections, and
 * sustained 429/5xx conditions would exhaust the mobile fetch connection pool.
 *
 * Fix: Add `await listRes.body?.cancel().catch(() => {})` before `continue`
 * in both the list-fetch retry path and the detail-fetch retry path, mirroring
 * the pattern already used in mobile/src/api/client.ts chat retry.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer } from '../../services/sync';

jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
  getNoteTimestamps: jest.fn(),
  getNoteTags: jest.fn(),
  addTag: jest.fn(),
  removeTag: jest.fn(),
  getPendingSyncs: jest.fn().mockResolvedValue([]),
}));

const mockGetNoteTimestamps = require('../../db').getNoteTimestamps as jest.MockedFunction<any>;

const mockFetch = jest.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

beforeEach(async () => {
  jest.clearAllMocks();
  await AsyncStorage.clear();
  mockFetch.mockReset();
  mockGetNoteTimestamps.mockResolvedValue([]);
  const mockGetNoteTags = require('../../db').getNoteTags as jest.MockedFunction<any>;
  mockGetNoteTags.mockResolvedValue([]);
});

/**
 * Build a Response-like object whose body.cancel is a spy, so we can assert
 * the sync retry path explicitly drains/cancels the body to prevent the
 * connection leak described in #3111.
 */
function mockResponseWithCancelableBody(
  status: number,
  body: unknown,
  headers: Record<string, string> = {}
) {
  const cancel = jest.fn().mockResolvedValue(undefined);
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (h: string) => headers[h.toLowerCase()] ?? null },
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(JSON.stringify(body)),
    // Mimic the Web Fetch API's ReadableStream body surface used by sync.ts.
    body: { cancel },
  };
}

describe('sync retry body cancellation (#3111)', () => {
  it('cancels the list-fetch response body before retrying on 429', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    const retryResponse = mockResponseWithCancelableBody(429, {
      error: 'rate limited',
    });
    const successResponse = mockResponseWithCancelableBody(200, {
      notes: [],
      total: 0,
      has_more: false,
    });
    mockFetch
      .mockResolvedValueOnce(retryResponse)
      .mockResolvedValueOnce(successResponse);

    await syncNotesFromServer();

    // Body of the 429 response must be cancelled to avoid connection leak.
    expect(retryResponse.body.cancel).toHaveBeenCalledTimes(1);
    // Success body is consumed downstream via .json() — must NOT be cancelled.
    expect(successResponse.body.cancel).not.toHaveBeenCalled();
  });

  it('cancels the detail-fetch response body before retrying on 503', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockGetNoteTimestamps.mockResolvedValue([]);

    // List succeeds with one note to fetch in detail.
    const listResponse = mockResponseWithCancelableBody(200, {
      notes: [{ id: 'note-1', title: 'T', updated_at: '2026-06-01T00:00:00Z' }],
      total: 1,
      has_more: false,
    });
    // First detail fetch -> 503 (retryable, must cancel body), second -> success.
    const detailRetryResponse = mockResponseWithCancelableBody(503, {
      error: 'unavailable',
    });
    const detailSuccessResponse = mockResponseWithCancelableBody(200, {
      meta: { id: 'note-1', title: 'T' },
      body: 'C',
    });
    mockFetch
      .mockResolvedValueOnce(listResponse)
      .mockResolvedValueOnce(detailRetryResponse)
      .mockResolvedValueOnce(detailSuccessResponse);

    const result = await syncNotesFromServer();

    expect(result.imported).toBe(1);
    expect(result.errors).toBe(0);
    // The 503 retry response body MUST be cancelled (#3111).
    expect(detailRetryResponse.body.cancel).toHaveBeenCalledTimes(1);
    // The success body is consumed via .json() downstream — must not be cancelled.
    expect(detailSuccessResponse.body.cancel).not.toHaveBeenCalled();
  });

  it('does NOT throw when retryable response body.cancel rejects', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    // body.cancel rejects — the .catch(() => {}) guard must swallow it so
    // retry still proceeds.
    const retryResponse = {
      ok: false,
      status: 429,
      headers: { get: () => null },
      json: () => Promise.resolve({ error: 'rate limited' }),
      text: () => Promise.resolve('rate limited'),
      body: { cancel: jest.fn().mockRejectedValue(new Error('already closed')) },
    };
    const successResponse = mockResponseWithCancelableBody(200, {
      notes: [],
      total: 0,
      has_more: false,
    });
    mockFetch
      .mockResolvedValueOnce(retryResponse)
      .mockResolvedValueOnce(successResponse);

    // Must not throw — cancel rejection is swallowed.
    await expect(syncNotesFromServer()).resolves.toBeDefined();
    expect(retryResponse.body.cancel).toHaveBeenCalledTimes(1);
  });
});
