/**
 * Regression test for sync detail-fetch body leak on non-retryable 4xx (#3119).
 *
 * Bug: The #3114 fix for #3111 only added `body?.cancel()` on the *retryable*
 * branch (429/502/503/504) of the detail-fetch loop. The non-retryable 4xx
 * branch (401/403/404/422) still did a bare `break`, leaving the response body
 * uncancelled. After the loop the failed-detail path returns without draining
 * either, so the underlying HTTP connection leaks back into nowhere.
 *
 * With DETAIL_CONCURRENCY = 5, a batch of 404/403 responses (e.g. notes
 * deleted server-side between list and detail fetch, or permission revocations)
 * leaks up to 5 connections per sync round and eventually exhausts the mobile
 * fetch connection pool — the same failure mode described in #3111.
 *
 * Fix: Add `await noteRes.body?.cancel().catch(() => {})` before `break` in
 * the 4xx branch, mirroring the retryable branch's handling.
 *
 * Source: mobile/src/services/sync.ts:271-275.
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

function mockResponseWithCancelableBody(
  status: number,
  body: unknown,
  headers: Record<string, string> = {},
) {
  const cancel = jest.fn().mockResolvedValue(undefined);
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (h: string) => headers[h.toLowerCase()] ?? null },
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(JSON.stringify(body)),
    body: { cancel },
  };
}

describe('sync detail-fetch 4xx body cancellation (#3119)', () => {
  it('cancels the detail-fetch response body on 404 (non-retryable)', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    // List succeeds with one note, then detail fetch returns 404 (note deleted
    // server-side between list and detail). The 404 is non-retryable so sync
    // must NOT retry — single detail fetch, body cancelled.
    const listResponse = mockResponseWithCancelableBody(200, {
      notes: [{ id: 'note-1', title: 'T', updated_at: '2026-06-01T00:00:00Z' }],
      total: 1,
      has_more: false,
    });
    const detail404Response = mockResponseWithCancelableBody(404, {
      error: 'not found',
    });

    mockFetch
      .mockResolvedValueOnce(listResponse)
      .mockResolvedValueOnce(detail404Response);

    const result = await syncNotesFromServer();

    // The 404 detail fetch counts as an error.
    expect(result.errors).toBe(1);
    expect(result.imported).toBe(0);
    // Critical: the 404 body MUST be cancelled to avoid connection leak (#3119).
    expect(detail404Response.body.cancel).toHaveBeenCalledTimes(1);
    // Only 2 fetches total: 1 list + 1 detail (no retry on 4xx).
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it('cancels the detail-fetch response body on 403 (non-retryable)', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    const listResponse = mockResponseWithCancelableBody(200, {
      notes: [{ id: 'note-1', title: 'T', updated_at: '2026-06-01T00:00:00Z' }],
      total: 1,
      has_more: false,
    });
    const detail403Response = mockResponseWithCancelableBody(403, {
      error: 'forbidden',
    });

    mockFetch
      .mockResolvedValueOnce(listResponse)
      .mockResolvedValueOnce(detail403Response);

    const result = await syncNotesFromServer();

    expect(result.errors).toBe(1);
    expect(detail403Response.body.cancel).toHaveBeenCalledTimes(1);
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it('does NOT throw when 4xx body.cancel rejects', async () => {
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    // The .catch(() => {}) guard must swallow cancel rejection so sync
    // proceeds to emit the error and return cleanly.
    const listResponse = mockResponseWithCancelableBody(200, {
      notes: [{ id: 'note-1', title: 'T', updated_at: '2026-06-01T00:00:00Z' }],
      total: 1,
      has_more: false,
    });
    const detail404Response = {
      ok: false,
      status: 404,
      headers: { get: () => null },
      json: () => Promise.resolve({ error: 'not found' }),
      text: () => Promise.resolve('not found'),
      body: { cancel: jest.fn().mockRejectedValue(new Error('already closed')) },
    };

    mockFetch
      .mockResolvedValueOnce(listResponse)
      .mockResolvedValueOnce(detail404Response);

    // Must not throw — cancel rejection is swallowed.
    await expect(syncNotesFromServer()).resolves.toBeDefined();
    expect(detail404Response.body.cancel).toHaveBeenCalledTimes(1);
  });

  it('cancels body for every 4xx detail in a concurrent batch (no leak stack-up)', async () => {
    // DETAIL_CONCURRENCY = 5 — simulate a batch where ALL 5 detail fetches
    // return 404. Every response body must be cancelled; otherwise 5
    // connections leak in a single sync round (the original #3119 impact).
    (AsyncStorage.getItem as jest.Mock)
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    const notes = Array.from({ length: 5 }, (_, i) => ({
      id: `note-${i}`,
      title: `T${i}`,
      updated_at: '2026-06-01T00:00:00Z',
    }));
    const listResponse = mockResponseWithCancelableBody(200, {
      notes,
      total: 5,
      has_more: false,
    });

    const detail404Responses = notes.map(() =>
      mockResponseWithCancelableBody(404, { error: 'not found' }),
    );

    mockFetch.mockResolvedValueOnce(listResponse);
    for (const r of detail404Responses) {
      mockFetch.mockResolvedValueOnce(r);
    }

    const result = await syncNotesFromServer();

    expect(result.errors).toBe(5);
    expect(result.imported).toBe(0);
    // Every 404 response body must have been cancelled exactly once.
    for (let i = 0; i < detail404Responses.length; i++) {
      expect(detail404Responses[i].body.cancel).toHaveBeenCalledTimes(1);
    }
  });
});
