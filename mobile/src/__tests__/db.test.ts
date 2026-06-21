import * as sqliteMock from 'expo-sqlite';
const mockDb = (sqliteMock as any).__getMockDb();

/** Get a fresh db module and initialize it (runs CREATE TABLE etc), then clear mock state. */
async function freshDb() {
  jest.resetModules();
  const freshSqlite = require('expo-sqlite');
  freshSqlite.openDatabaseAsync.mockResolvedValue(mockDb);
  const db = require('../db');
  // Trigger getDb() to initialize the database
  await db.getDb();
  // Clear all mock calls from initialization so tests start clean
  jest.clearAllMocks();
  return db;
}

describe('db CRUD operations', () => {
  it('createSession returns a uuid-like string', async () => {
    const db = await freshDb();
    const id = await db.createSession('test');
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
  });

  it('getSessions queries with archived flag', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([
      { id: '1', title: 's1', created_at: 0, updated_at: 0, pinned: 0, archived: 0 },
    ]);
    const sessions = await db.getSessions(false);
    expect(sessions).toHaveLength(1);
    const [sql, params] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('archived = ?');
    expect(params).toEqual([0]);
  });

  it('getSessions with archived=true queries archived=1', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.getSessions(true);
    const [, params] = mockDb.getAllAsync.mock.calls[0];
    expect(params).toEqual([1]);
  });

  it('addMessage calls withTransactionAsync', async () => {
    const db = await freshDb();
    const id = await db.addMessage('sess1', 'user', 'hello');
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
    expect(mockDb.withTransactionAsync).toHaveBeenCalled();
  });

  it('createNote returns uuid', async () => {
    const db = await freshDb();
    const id = await db.createNote('My Note');
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
  });

  it('getNotes with folder filter passes folder param', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.getNotes('work');
    const [sql, params] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('folder = ?');
    expect(params).toEqual(['work']);
  });

  it('getNotes without folder returns all', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.getNotes();
    const [sql] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).not.toContain('folder = ?');
  });

  it('renameSession updates title', async () => {
    const db = await freshDb();
    await db.renameSession('id1', 'New Title');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('UPDATE sessions SET title');
    expect(params[0]).toBe('New Title');
    expect(params[1]).toBe('id1');
  });

  it('deleteSession deletes by id', async () => {
    const db = await freshDb();
    await db.deleteSession('id1');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('DELETE FROM sessions');
    expect(params).toEqual(['id1']);
  });
});

describe('searchSessions', () => {
  it('returns results from getAllAsync', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([
      { id: '1', title: 'test', created_at: 0, updated_at: 0, pinned: 0, archived: 0 },
    ]);
    const results = await db.searchSessions('test');
    expect(Array.isArray(results)).toBe(true);
    expect(results).toHaveLength(1);
  });

  it('escapes LIKE special characters in search query', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([]);
    await db.searchSessions('100%_done');
    // Find the call that contains the LIKE pattern
    const calls = mockDb.getAllAsync.mock.calls;
    const searchCall = calls.find((c: any[]) => typeof c[0] === 'string' && c[0].includes('LIKE'));
    expect(searchCall).toBeDefined();
    const params = searchCall![1];
    const pattern = params[params.length - 1];
    expect(pattern).toContain('100');
  });
});

describe('tag operations', () => {
  it('addTag uses INSERT OR IGNORE', async () => {
    const db = await freshDb();
    await db.addTag('note1', 'important');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('INSERT OR IGNORE');
    expect(params).toEqual(['note1', 'important']);
  });

  it('removeTag deletes by note_id and tag', async () => {
    const db = await freshDb();
    await db.removeTag('note1', 'important');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('DELETE FROM note_tags');
    expect(params).toEqual(['note1', 'important']);
  });

  it('getAllTags returns distinct tags', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([{ tag: 'a' }, { tag: 'b' }]);
    const tags = await db.getAllTags();
    expect(tags).toEqual(['a', 'b']);
  });

  it('getNoteTags returns tags for a note', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([{ tag: 'x' }]);
    const tags = await db.getNoteTags('note1');
    expect(tags).toEqual(['x']);
  });
});

describe('toggleStar', () => {
  it('flips starred between 0 and 1', async () => {
    const db = await freshDb();
    await db.toggleStar('note1');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('1 - starred');
    expect(params).toEqual(['note1']);
  });
});

describe('togglePin / toggleArchive', () => {
  it('togglePin flips pinned', async () => {
    const db = await freshDb();
    await db.togglePin('sess1');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('1 - pinned');
    expect(params).toEqual(['sess1']);
  });

  it('toggleArchive flips archived', async () => {
    const db = await freshDb();
    await db.toggleArchive('sess1');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('1 - archived');
    expect(params).toEqual(['sess1']);
  });
});

describe('getDb singleton', () => {
  it('retries on failure (resets singleton)', async () => {
    jest.resetModules();
    const freshSqlite = require('expo-sqlite');
    freshSqlite.openDatabaseAsync
      .mockRejectedValueOnce(new Error('disk full'))
      .mockResolvedValueOnce(mockDb);
    const db = require('../db');

    await expect(db.getDb()).rejects.toThrow('disk full');
    await db.getDb();
    expect(freshSqlite.openDatabaseAsync).toHaveBeenCalledTimes(2);
  });
});

describe('moveToFolder', () => {
  it('updates folder column', async () => {
    const db = await freshDb();
    await db.moveToFolder('note1', 'projects');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('UPDATE notes SET folder');
    expect(params[0]).toBe('projects');
    expect(params[1]).toBe('note1');
  });
});

describe('getMessages', () => {
  it('returns messages ordered by created_at', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([
      { id: '1', session_id: 's1', role: 'user', content: 'Hi' },
      { id: '2', session_id: 's1', role: 'assistant', content: 'Hello!' },
    ]);
    const msgs = await db.getMessages('s1');
    expect(msgs).toHaveLength(2);
    const [sql, params] = mockDb.getAllAsync.mock.calls[0];
    expect(sql).toContain('ORDER BY created_at');
    expect(params).toEqual(['s1']);
  });
});

describe('updateMessage', () => {
  it('updates message content', async () => {
    const db = await freshDb();
    await db.updateMessage('msg1', 'Updated');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('UPDATE messages SET content');
    expect(params).toEqual(['Updated', 'msg1']);
  });
});

describe('deleteMessage', () => {
  it('deletes message by id', async () => {
    const db = await freshDb();
    await db.deleteMessage('msg1');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('DELETE FROM messages');
    expect(params).toEqual(['msg1']);
  });
});

describe('getNote', () => {
  it('returns note by id', async () => {
    const db = await freshDb();
    mockDb.getFirstAsync.mockResolvedValueOnce({
      id: 'n1', title: 'Test', content: 'Body', starred: 0, folder: '',
    });
    const note = await db.getNote('n1');
    expect(note).toBeTruthy();
    expect(note!.title).toBe('Test');
  });
});

describe('updateNote', () => {
  it('updates title and content', async () => {
    const db = await freshDb();
    await db.updateNote('n1', 'New Title', 'New Body');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('UPDATE notes SET title');
    expect(params[0]).toBe('New Title');
    expect(params[1]).toBe('New Body');
    expect(params[2]).toBe('n1');
  });
});

describe('deleteNote', () => {
  it('deletes note by id', async () => {
    const db = await freshDb();
    await db.deleteNote('n1');
    const [sql, params] = mockDb.runAsync.mock.calls[0];
    expect(sql).toContain('DELETE FROM notes');
    expect(params).toEqual(['n1']);
  });
});

describe('getFolders', () => {
  it('returns non-empty folder names', async () => {
    const db = await freshDb();
    mockDb.getAllAsync.mockResolvedValueOnce([{ folder: 'work' }, { folder: 'archive' }]);
    const folders = await db.getFolders();
    expect(folders).toEqual(['work', 'archive']);
  });
});

describe('globalSearch', () => {
  it('returns empty for empty query', async () => {
    const db = await freshDb();
    const results = await db.globalSearch('');
    expect(results).toEqual([]);
  });

  it('returns note and session results for non-empty query', async () => {
    const db = await freshDb();
    // First call: FTS note search (returns results)
    mockDb.getAllAsync
      .mockResolvedValueOnce([{ type: 'note', id: 'n1', title: 'Test Note', snippet: 'content...', updated_at: 2000 }])
      // Second call: FTS session message search
      .mockResolvedValueOnce([{ type: 'session', id: 'm1', title: 'Chat', snippet: 'message...', updated_at: 1000, sessionId: 's1' }]);

    const results = await db.globalSearch('test');
    expect(results).toHaveLength(2);
    expect(results[0].type).toBe('note');
    expect(results[1].type).toBe('session');
  });

  it('falls back to LIKE when FTS note search returns empty', async () => {
    const db = await freshDb();
    // FTS note search returns empty
    mockDb.getAllAsync
      .mockResolvedValueOnce([])
      // LIKE fallback for notes
      .mockResolvedValueOnce([{ type: 'note', id: 'n2', title: 'CJK Note', snippet: 'content', updated_at: 500 }])
      // FTS session search
      .mockResolvedValueOnce([]);

    const results = await db.globalSearch('CJK');
    expect(results).toHaveLength(1);
    expect(results[0].id).toBe('n2');
  });
});
