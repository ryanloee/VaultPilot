/**
 * Regression test for db.ts FTS query deduplication (#1392).
 *
 * Tests the buildFtsQuery helper extracted from 4 duplicate inline
 * FTS5 MATCH query constructions in searchSessions, searchNotes,
 * and globalSearch.
 */

import { searchNotes, searchSessions, globalSearch } from '../../db';
import { __getMockDb } from '../../__mocks__/expo-sqlite';

const mockDb = __getMockDb();

beforeEach(() => {
  jest.clearAllMocks();
  mockDb.getAllAsync.mockResolvedValue([]);
  mockDb.getFirstAsync.mockResolvedValue({ c: 0 });
  mockDb.execAsync.mockResolvedValue(undefined);
});

describe('FTS query building (#1392)', () => {
  it('searchNotes builds correct FTS query with quoted terms', async () => {
    await searchNotes('hello world');
    const ftsCall = mockDb.getAllAsync.mock.calls.find(
      (call: any[]) => typeof call[0] === 'string' && call[0].includes('fts MATCH')
    );
    expect(ftsCall).toBeDefined();
    expect(ftsCall![1][0]).toBe('"hello" OR "world"');
  });

  it('searchNotes escapes double quotes in FTS query', async () => {
    await searchNotes('say "hello"');
    const ftsCall = mockDb.getAllAsync.mock.calls.find(
      (call: any[]) => typeof call[0] === 'string' && call[0].includes('fts MATCH')
    );
    expect(ftsCall).toBeDefined();
    expect(ftsCall![1][0]).toBe('"say" OR """hello"""');
  });

  it('searchSessions builds correct FTS query', async () => {
    await searchSessions('test query');
    const ftsCall = mockDb.getAllAsync.mock.calls.find(
      (call: any[]) => typeof call[0] === 'string' && call[0].includes('fts MATCH')
    );
    expect(ftsCall).toBeDefined();
    expect(ftsCall![1][0]).toBe('"test" OR "query"');
  });

  it('globalSearch builds FTS queries for both notes and sessions', async () => {
    await globalSearch('search term');
    const ftsCalls = mockDb.getAllAsync.mock.calls.filter(
      (call: any[]) => typeof call[0] === 'string' && call[0].includes('MATCH')
    );
    expect(ftsCalls.length).toBe(2);
    expect(ftsCalls[0][1][0]).toBe('"search" OR "term"');
    expect(ftsCalls[1][1][0]).toBe('"search" OR "term"');
  });
});
