/**
 * Regression test for sync 429 retry behavior (#2132).
 *
 * Bug: syncNotesFromServer used a hardcoded `status >= 500` to decide whether
 * to retry transient failures on both the note-list fetch and the note-detail
 * fetch. A 429 Too Many Requests was therefore treated as a non-retryable 4xx
 * — the list fetch failed the whole sync immediately, and a detail fetch
 * silently skipped the note — even though the project-wide `isRetryable()`
 * helper explicitly includes 429.
 *
 * Fix: replace `status >= 500` with `isRetryable(status)` so 429 (and
 * 502/503/504) go through the existing exponential backoff, optionally
 * honoring a `Retry-After` response header.
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import { syncNotesFromServer, parseRetryAfter } from '../../services/sync';

jest.mock('../../db', () => ({
  createNote: jest.fn(),
  updateNote: jest.fn(),
  getNote: jest.fn(),
  getNotes: jest.fn(),
  getNoteTimestamps: jest.fn(),
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
});

/** Build a Response-like object with the given status and optional headers. */
function mockResponse(status: number, body: unknown, headers: Record<string, string> = {}) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (h: string) => headers[h.toLowerCase()] ?? null },
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(JSON.stringify(body)),
  };
}

describe('sync 429 retry behavior (#2132)', () => {
  it('retries the note-list fetch on 429 instead of failing immediately', async () => {
    (
      AsyncStorage.getItem as jest.Mock
    )
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    // First list attempt -> 429 (should retry), second attempt -> success.
    mockFetch
      .mockResolvedValueOnce(mockResponse(429, { error: 'rate limited' }))
      .mockResolvedValueOnce(
        mockResponse(200, { notes: [], total: 0, has_more: false })
      );

    const result = await syncNotesFromServer();

    expect(result.imported).toBe(0);
    expect(result.errors).toBe(0);
    // Two list fetches: the 429 + the successful retry.
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it('does NOT treat 404 as retryable on the note-list fetch', async () => {
    (
      AsyncStorage.getItem as jest.Mock
    )
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    // 404 is non-retryable: must surface as a thrown error, not silently retry.
    mockFetch.mockResolvedValueOnce(mockResponse(404, { error: 'not found' }));

    await expect(syncNotesFromServer()).rejects.toThrow();
    // Only one list fetch — no retry for a non-retryable status.
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('honors Retry-After (seconds) header between 429 retries', async () => {
    (
      AsyncStorage.getItem as jest.Mock
    )
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');

    mockFetch
      .mockResolvedValueOnce(
        mockResponse(429, { error: 'rate limited' }, { 'retry-after': '2' })
      )
      .mockResolvedValueOnce(
        mockResponse(200, { notes: [], total: 0, has_more: false })
      );

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const sleepSpy = jest.spyOn(globalThis, 'setTimeout') as unknown as {
      mock: { calls: number[][] };
      mockRestore: () => void;
    };
    await syncNotesFromServer();

    // At least one setTimeout used a delay in the ~2000ms range (Retry-After=2s).
    // setTimeout signature is (callback, delay) -> delay is the second argument.
    const delays = sleepSpy.mock.calls.map((c) => c[1]);
    expect(delays.some((d) => d >= 1500 && d <= 2500)).toBe(true);
    sleepSpy.mockRestore();
  });

  it('retries note-detail fetch on 429 instead of skipping the note', async () => {
    (
      AsyncStorage.getItem as jest.Mock
    )
      .mockResolvedValueOnce('http://localhost:3000')
      .mockResolvedValueOnce('');
    mockGetNoteTimestamps.mockResolvedValue([]);

    // List returns one note to fetch in detail.
    mockFetch.mockResolvedValueOnce(
      mockResponse(200, {
        notes: [
          { id: 'note-1', title: 'T', updated_at: '2026-06-01T00:00:00Z' },
        ],
        total: 1,
        has_more: false,
      })
    );
    // First detail fetch -> 429 (should retry), second detail fetch -> success.
    mockFetch
      .mockResolvedValueOnce(mockResponse(429, { error: 'rate limited' }))
      .mockResolvedValueOnce(
        mockResponse(200, {
          meta: { id: 'note-1', title: 'T' },
          body: 'C',
        })
      );

    const result = await syncNotesFromServer();
    expect(result.imported).toBe(1);
    expect(result.errors).toBe(0);
  });
});

describe('parseRetryAfter (#2132)', () => {
  const makeRes = (headers: Record<string, string>) =>
    ({
      headers: { get: (h: string) => headers[h.toLowerCase()] ?? null },
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);

  it('parses a numeric seconds value', () => {
    expect(parseRetryAfter(makeRes({ 'retry-after': '30' }))).toBe(30000);
  });

  it('parses an HTTP-date value relative to now', () => {
    const future = new Date(Date.now() + 5000).toUTCString();
    const ms = parseRetryAfter(makeRes({ 'retry-after': future }));
    expect(ms).not.toBeNull();
    expect(ms!).toBeGreaterThan(0);
    expect(ms!).toBeLessThanOrEqual(5000 + 1000);
  });

  it('returns null when header is absent', () => {
    expect(parseRetryAfter(makeRes({}))).toBeNull();
  });

  it('caps the delay at a sane maximum', () => {
    expect(parseRetryAfter(makeRes({ 'retry-after': '999999' }))).toBeLessThanOrEqual(
      60000
    );
  });
});
