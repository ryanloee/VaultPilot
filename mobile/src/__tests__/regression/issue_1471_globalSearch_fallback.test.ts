/**
 * Regression test for #1471: globalSearch FTS→LIKE fallback.
 *
 * When FTS returns empty results (common with CJK text), globalSearch
 * should fall back to LIKE search, matching searchNotes behavior.
 */
import * as sqliteMock from 'expo-sqlite';
const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db module and initialize it, then clear mock state. */
async function freshDb() {
  jest.resetModules();
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);
  const db = require('../../db');
  await db.getDb();
  jest.clearAllMocks();
  return db;
}

describe('globalSearch FTS→LIKE fallback (#1471)', () => {
  it('falls back to LIKE when FTS MATCH returns empty for notes', async () => {
    const db = await freshDb();
    let getAllCalls = 0;
    mockDb.getAllAsync.mockImplementation((sql: string, params?: any[]) => {
      getAllCalls++;
      if (sql.includes('notes_fts') && sql.includes('MATCH')) return Promise.resolve([]);
      if (sql.includes('notes') && sql.includes('LIKE')) {
        return Promise.resolve([
          { type: 'note', id: 'n1', title: '测试笔记', snippet: '内容', updated_at: 100 },
        ]);
      }
      if (sql.includes('messages_fts') && sql.includes('MATCH')) return Promise.resolve([]);
      if (sql.includes('messages') && sql.includes('LIKE')) return Promise.resolve([]);
      return Promise.resolve([]);
    });

    const results = await db.globalSearch('测试');
    const noteResults = results.filter((r: any) => r.type === 'note');
    expect(noteResults.length).toBe(1);
    expect(noteResults[0].id).toBe('n1');
  });

  it('falls back to LIKE when FTS MATCH returns empty for sessions', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockImplementation((sql: string) => {
      if (sql.includes('notes_fts') && sql.includes('MATCH')) return Promise.resolve([]);
      if (sql.includes('notes') && sql.includes('LIKE')) return Promise.resolve([]);
      if (sql.includes('messages_fts') && sql.includes('MATCH')) return Promise.resolve([]);
      if (sql.includes('messages') && sql.includes('LIKE')) {
        return Promise.resolve([
          { type: 'session', id: 'm1', title: '对话', snippet: '你好世界', updated_at: 200, sessionId: 's1' },
        ]);
      }
      return Promise.resolve([]);
    });

    const results = await db.globalSearch('你好');
    const sessionResults = results.filter((r: any) => r.type === 'session');
    expect(sessionResults.length).toBe(1);
    expect(sessionResults[0].sessionId).toBe('s1');
  });

  it('does NOT fall back when FTS returns results', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockImplementation((sql: string) => {
      if (sql.includes('notes_fts') && sql.includes('MATCH')) {
        return Promise.resolve([{ type: 'note', id: 'n1', title: 'Test', snippet: 'content', updated_at: 100 }]);
      }
      if (sql.includes('notes') && sql.includes('LIKE')) {
        // Should NOT be reached
        return Promise.resolve([{ type: 'note', id: 'n2', title: 'Nope', snippet: '', updated_at: 50 }]);
      }
      if (sql.includes('messages_fts') && sql.includes('MATCH')) {
        return Promise.resolve([{ type: 'session', id: 'm1', title: 'Chat', snippet: 'hi', updated_at: 200, sessionId: 's1' }]);
      }
      if (sql.includes('messages') && sql.includes('LIKE')) return Promise.resolve([]);
      return Promise.resolve([]);
    });

    const results = await db.globalSearch('Test');
    const noteIds = results.filter((r: any) => r.type === 'note').map((r: any) => r.id);
    expect(noteIds).toEqual(['n1']);
    expect(noteIds).not.toContain('n2');
  });
});
