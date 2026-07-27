/**
 * Regression test for #3497: FTS session search missing title LIKE.
 *
 * Before the fix, the FTS path in globalSearch only searched message content
 * via `fts MATCH ?` without also searching session titles. This meant FTS users
 * couldn't find sessions by title — inconsistent with the LIKE fallback which
 * correctly uses `m.content LIKE ? OR s.title LIKE ?`.
 *
 * Fix: added `OR s.title LIKE ? ESCAPE '\\'` to the FTS session query.
 */

import * as sqliteMock from 'expo-sqlite';
const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db with FTS enabled. */
async function freshDbWithFts() {
  jest.resetModules();
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);

  // Make FTS creation succeed
  mockDb.execAsync.mockImplementation((_sql: string) => Promise.resolve(undefined));

  const db = require('../../db');
  await db.getDb();
  jest.clearAllMocks();
  return db;
}

describe('globalSearch FTS path — session title (#3497)', () => {
  it('FTS session query includes s.title LIKE alongside fts MATCH (4 params)', async () => {
    const db = await freshDbWithFts();

    // FTS is available, so globalSearch uses MATCH for notes + MATCH for sessions
    // getAllAsync calls: FTS notes, FTS sessions
    mockDb.getAllAsync
      .mockResolvedValueOnce([{ type: 'note', id: 'n1', title: 'Test', snippet: 'content', updated_at: 100 }])
      .mockResolvedValueOnce([{ type: 'session', id: 'm1', title: 'Chat', snippet: 'msg', updated_at: 50, sessionId: 's1' }]);

    const results = await db.globalSearch('test');
    expect(results).toHaveLength(2);

    // Find the FTS session query (contains 's.title' AND 'messages_fts')
    const ftsSessionCall = mockDb.getAllAsync.mock.calls.find(
      (call: unknown[]) =>
        typeof call[0] === 'string' &&
        (call[0] as string).includes('messages_fts') &&
        (call[0] as string).includes('s.title')
    );
    expect(ftsSessionCall).toBeDefined();

    // The SQL should include both MATCH and title LIKE
    const sql = ftsSessionCall![0] as string;
    expect(sql).toContain('fts MATCH ?');
    expect(sql).toContain("s.title LIKE ? ESCAPE '\\'");

    // 3 params: ftsQuery, title LIKE pattern, limit
    expect(ftsSessionCall![1]).toHaveLength(3);
    expect(ftsSessionCall![1][1]).toBe('%test%');
    expect(ftsSessionCall![1][2]).toBe(20);
  });
});