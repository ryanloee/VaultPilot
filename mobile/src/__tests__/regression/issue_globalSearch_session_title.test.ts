/**
 * Regression test for globalSearch LIKE fallback: session title search.
 *
 * When FTS is unavailable, the LIKE fallback for sessions should search
 * both message content AND session titles (not just content).
 *
 * Fix: added `OR s.title LIKE ?` to the session LIKE query.
 */

import * as sqliteMock from 'expo-sqlite';
const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db with FTS disabled. */
async function freshDbNoFts() {
  jest.resetModules();
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);

  // Make FTS creation fail so ftsSupported = false
  mockDb.execAsync.mockImplementation((sql: string) => {
    if (sql.includes('fts5') || sql.includes('FTS5')) {
      return Promise.reject(new Error('FTS5 not supported'));
    }
    return Promise.resolve(undefined);
  });

  const db = require('../../db');
  await db.getDb();
  jest.clearAllMocks();
  return db;
}

describe('globalSearch session title LIKE fallback', () => {
  it('LIKE fallback for sessions searches both content and title (3 params)', async () => {
    const db = await freshDbNoFts();

    // With FTS disabled, globalSearch uses LIKE for both notes and sessions
    // getAllAsync calls: LIKE notes, LIKE sessions
    mockDb.getAllAsync
      .mockResolvedValueOnce([{ type: 'note', id: 'n1', title: 'Test', snippet: 'content', updated_at: 100 }])
      .mockResolvedValueOnce([{ type: 'session', id: 'm1', title: 'Chat', snippet: 'msg', updated_at: 50, sessionId: 's1' }]);

    const results = await db.globalSearch('test');
    expect(results).toHaveLength(2);

    // Find the session LIKE query (contains 's.title')
    const sessionLikeCall = mockDb.getAllAsync.mock.calls.find(
      (call: unknown[]) => typeof call[0] === 'string' && (call[0] as string).includes('s.title')
    );
    expect(sessionLikeCall).toBeDefined();
    // 3 params: content LIKE pattern, title LIKE pattern, limit
    expect(sessionLikeCall![1]).toHaveLength(3);
    expect(sessionLikeCall![1][0]).toBe('%test%');
    expect(sessionLikeCall![1][1]).toBe('%test%');
    expect(sessionLikeCall![1][2]).toBe(20);
  });
});
