/**
 * Tests for #2042: 笔记多集合归属 — many-to-many note <-> collection membership.
 *
 * Covers the collection DB API surface added to mobile/src/db.ts:
 *   getCollections / getCollectionsWithCounts / getNoteCollections /
 *   addNoteToCollection / removeNoteFromCollection / setNoteCollections /
 *   getNotesInCollection / searchNotesInCollection / parseCollections.
 *
 * Uses the shared expo-sqlite mock (see __mocks__/expo-sqlite.ts) and the same
 * freshDb() reset pattern as db.test.ts.
 */
import * as sqliteMock from 'expo-sqlite';
const mockDb = (sqliteMock as any).__getMockDb();

/** Reset the db module, initialize it (runs CREATE TABLE), then clear mock state. */
async function freshDb() {
  jest.resetModules();
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);
  const db = require('../../db');
  await db.getDb();
  jest.clearAllMocks();
  return db;
}

describe('issue #2042 — collections (many-to-many note membership)', () => {
  describe('parseCollections (pure helper)', () => {
    it('splits, trims and dedupes a GROUP_CONCAT csv', () => {
      const { parseCollections } = require('../../db');
      expect(parseCollections('work,work, projectA ,')).toEqual(['work', 'projectA']);
    });
    it('returns [] for empty/undefined', () => {
      const { parseCollections } = require('../../db');
      expect(parseCollections(undefined)).toEqual([]);
      expect(parseCollections('')).toEqual([]);
    });
  });

  it('addNoteToCollection issues INSERT OR IGNORE with (note_id, collection)', async () => {
    const db = await freshDb();
    await db.addNoteToCollection('note-1', 'Work');
    expect(mockDb.runAsync).toHaveBeenCalledWith(
      expect.stringContaining('INSERT OR IGNORE INTO note_collections'),
      ['note-1', 'Work']
    );
  });

  it('addNoteToCollection ignores empty / whitespace-only names (no DB write)', async () => {
    const db = await freshDb();
    await db.addNoteToCollection('note-1', '   ');
    expect(mockDb.runAsync).not.toHaveBeenCalled();
  });

  it('removeNoteFromCollection issues DELETE scoped to (note_id, collection)', async () => {
    const db = await freshDb();
    await db.removeNoteFromCollection('note-1', 'Work');
    expect(mockDb.runAsync).toHaveBeenCalledWith(
      expect.stringContaining('DELETE FROM note_collections'),
      ['note-1', 'Work']
    );
    const sql = mockDb.runAsync.mock.calls[0][0] as string;
    expect(sql).toMatch(/note_id\s*=\s*\?/);
    expect(sql).toMatch(/collection\s*=\s*\?/);
  });

  it('getCollections returns DISTINCT collection names sorted', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([
      { collection: 'Work' }, { collection: 'Personal' },
    ]);
    const res = await db.getCollections();
    expect(res).toEqual(['Work', 'Personal']);
    const [sql] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('SELECT DISTINCT collection FROM note_collections');
    expect(sql).toContain('ORDER BY collection');
  });

  it('getCollectionsWithCounts groups by collection and counts', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([
      { collection: 'Work', count: 3 }, { collection: 'Personal', count: 1 },
    ]);
    const res = await db.getCollectionsWithCounts();
    expect(res).toEqual([{ collection: 'Work', count: 3 }, { collection: 'Personal', count: 1 }]);
    const [sql] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('COUNT(*)');
    expect(sql).toContain('GROUP BY collection');
  });

  it('getNoteCollections filters by a single note_id', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([{ collection: 'Work' }]);
    const res = await db.getNoteCollections('note-1');
    expect(res).toEqual(['Work']);
    const [sql, params] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('WHERE note_id = ?');
    expect(params).toEqual(['note-1']);
  });

  it('setNoteCollections atomically replaces all memberships (delete + insert, in a transaction)', async () => {
    const db = await freshDb();
    await db.setNoteCollections('note-1', ['Work', 'Personal']);
    expect(mockDb.withTransactionAsync).toHaveBeenCalled();
    // The set must be deduped + sorted
    const inserts = mockDb.runAsync.mock.calls.filter(
      (c: any[]) => typeof c[0] === 'string' && c[0].includes('INSERT OR IGNORE INTO note_collections')
    );
    expect(inserts).toHaveLength(2);
    expect(inserts[0][1]).toEqual(['note-1', 'Personal']); // sorted
    expect(inserts[1][1]).toEqual(['note-1', 'Work']);
    const deleteCall = mockDb.runAsync.mock.calls.find(
      (c: any[]) => typeof c[0] === 'string' && c[0].includes('DELETE FROM note_collections WHERE note_id = ?')
    );
    expect(deleteCall).toBeDefined();
    expect(deleteCall![1]).toEqual(['note-1']);
  });

  it('setNoteCollections clears memberships when given an empty list', async () => {
    const db = await freshDb();
    await db.setNoteCollections('note-1', []);
    const inserts = mockDb.runAsync.mock.calls.filter(
      (c: any[]) => typeof c[0] === 'string' && c[0].includes('INSERT OR IGNORE INTO note_collections')
    );
    expect(inserts).toHaveLength(0);
    expect(mockDb.withTransactionAsync).toHaveBeenCalled();
  });

  it('getNotesInCollection(collection) joins note_collections and filters by collection', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.getNotesInCollection('Work');
    const [sql, params] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('FROM notes n');
    expect(sql).toContain('INNER JOIN note_collections c ON c.note_id = n.id');
    expect(sql).toContain('c.collection = ?');
    expect(params).toEqual(['Work']);
    // must carry collections_csv subquery for badge rendering
    expect(sql).toContain('collections_csv');
  });

  it('getNotesInCollection(undefined) lists all notes with collections_csv', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.getNotesInCollection(undefined);
    const call = mockDb.getAllAsync.mock.calls[0];
    const sql = call[0] as string;
    expect(sql).toContain('collections_csv');
    expect(sql).not.toContain('c.collection = ?');
    // no collection filter and no limit => getAllAsync called with just the SQL
    expect(call.length).toBe(1);
  });

  it('searchNotesInCollection(query, collection) scopes the search to a collection', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]); // fts miss -> fallback also empty
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.searchNotesInCollection('keyword', 'Work');
    // at least one call must scope by collection
    const scopedCall = mockDb.getAllAsync.mock.calls.find((c: any[]) => {
      const sql = c[0] as string;
      return sql.includes('c.collection = ?') && c[1].includes('Work');
    });
    expect(scopedCall).toBeDefined();
  });
});
