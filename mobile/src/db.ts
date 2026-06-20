import * as SQLite from 'expo-sqlite';

let dbPromise: Promise<SQLite.SQLiteDatabase> | null = null;

/** Ensure columns added after the initial schema exist on existing installs. */
async function migrateSchema(db: SQLite.SQLiteDatabase): Promise<void> {
  const columns = async (table: string): Promise<Set<string>> => {
    const info = await db.getAllAsync<{ name: string }>(`PRAGMA table_info(${table})`);
    return new Set(info.map(c => c.name));
  };

  const ensureColumn = async (table: string, col: string, decl: string) => {
    const cols = await columns(table);
    if (!cols.has(col)) {
      await db.execAsync(`ALTER TABLE ${table} ADD COLUMN ${col} ${decl}`);
    }
  };

  await ensureColumn('sessions', 'pinned', 'INTEGER DEFAULT 0');
  await ensureColumn('sessions', 'archived', 'INTEGER DEFAULT 0');
  await ensureColumn('notes', 'starred', 'INTEGER DEFAULT 0');
}

/** Populate FTS tables from existing data (runs once, idempotent via content= sync). */
async function migrateFts(db: SQLite.SQLiteDatabase): Promise<void> {
  // Rebuild FTS content from source tables — safe to run on every open
  // because content= tables auto-sync via triggers for new writes.
  // This only catches rows inserted before triggers were created.
  const msgCount = await db.getFirstAsync<{ c: number }>('SELECT COUNT(*) as c FROM messages_fts');
  const noteCount = await db.getFirstAsync<{ c: number }>('SELECT COUNT(*) as c FROM notes_fts');
  if (msgCount && msgCount.c === 0) {
    const msgs = await db.getAllAsync<{ rowid: number; content: string }>('SELECT rowid, content FROM messages');
    for (const m of msgs) {
      await db.runAsync('INSERT INTO messages_fts(rowid, content) VALUES (?, ?)', [m.rowid, m.content]);
    }
  }
  if (noteCount && noteCount.c === 0) {
    const notes = await db.getAllAsync<{ rowid: number; title: string; content: string }>('SELECT rowid, title, content FROM notes');
    for (const n of notes) {
      await db.runAsync('INSERT INTO notes_fts(rowid, title, content) VALUES (?, ?, ?)', [n.rowid, n.title, n.content]);
    }
  }
}

export async function getDb(): Promise<SQLite.SQLiteDatabase> {
  if (!dbPromise) {
    dbPromise = (async () => {
      const db = await SQLite.openDatabaseAsync('vaultpilot.db');
      await db.execAsync('PRAGMA foreign_keys = ON;');
      await db.execAsync(`
        CREATE TABLE IF NOT EXISTS sessions (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL DEFAULT '新对话',
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          pinned INTEGER DEFAULT 0,
          archived INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS notes (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL DEFAULT '无标题',
          content TEXT NOT NULL DEFAULT '',
          starred INTEGER DEFAULT 0,
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );
      `);
      await db.execAsync('CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);');
      // FTS5 virtual tables for full-text search
      await db.execAsync(`
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content, content=messages, content_rowid=rowid);
        CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(title, content, content=notes, content_rowid=rowid);
      `);
      // Triggers to keep FTS in sync
      await db.execAsync(`
        CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
          INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
          INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
          INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
          INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
          INSERT INTO notes_fts(rowid, title, content) VALUES (new.rowid, new.title, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
          INSERT INTO notes_fts(notes_fts, rowid, title, content) VALUES('delete', old.rowid, old.title, old.content);
        END;
        CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
          INSERT INTO notes_fts(notes_fts, rowid, title, content) VALUES('delete', old.rowid, old.title, old.content);
          INSERT INTO notes_fts(rowid, title, content) VALUES (new.rowid, new.title, new.content);
        END;
      `);
      await migrateSchema(db);
      await migrateFts(db);
      return db;
    })().catch(err => {
      dbPromise = null; // Reset so next call retries instead of caching the failure
      throw err;
    });
  }
  return dbPromise;
}

function uuid(): string {
  return crypto.randomUUID();
}

/** Escape SQL LIKE special characters (%, _, \) so they match literally. */
function escapeLikePattern(pattern: string): string {
  return pattern.replace(/[\\%_]/g, ch => `\\${ch}`);
}

export interface DbSession {
  id: string; title: string; created_at: number; updated_at: number; pinned: number; archived: number;
}

export async function createSession(title = '新对话'): Promise<string> {
  const db = await getDb();
  const id = uuid();
  await db.runAsync('INSERT INTO sessions (id, title) VALUES (?, ?)', [id, title]);
  return id;
}

export async function getLatestSession(): Promise<DbSession | null> {
  const db = await getDb();
  return db.getFirstAsync<DbSession>(
    'SELECT * FROM sessions WHERE archived = 0 ORDER BY updated_at DESC LIMIT 1'
  );
}

export async function getSessions(archived = false): Promise<DbSession[]> {
  const db = await getDb();
  return db.getAllAsync<DbSession>(
    'SELECT * FROM sessions WHERE archived = ? ORDER BY pinned DESC, updated_at DESC',
    [archived ? 1 : 0]
  );
}

export async function renameSession(id: string, title: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE sessions SET title = ?, updated_at = strftime(\'%s\',\'now\') WHERE id = ?', [title, id]);
}

export async function deleteSession(id: string): Promise<void> {
  const db = await getDb();
  // ON DELETE CASCADE on messages FK handles message cleanup
  await db.runAsync('DELETE FROM sessions WHERE id = ?', [id]);
}

export async function togglePin(id: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE sessions SET pinned = 1 - pinned WHERE id = ?', [id]);
}

export async function toggleArchive(id: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE sessions SET archived = 1 - archived WHERE id = ?', [id]);
}

export async function searchSessions(query: string): Promise<DbSession[]> {
  const db = await getDb();
  const ftsQuery = query.split(/\s+/).filter(Boolean).map(t => `"${t}"`).join(' OR ');
  if (!ftsQuery) return [];
  const escaped = escapeLikePattern(query);
  // FTS5 on message content + LIKE on session title (titles are short, LIKE is fine)
  return db.getAllAsync<DbSession>(
    `SELECT DISTINCT s.* FROM sessions s
     LEFT JOIN messages m ON s.id = m.session_id
     LEFT JOIN messages_fts fts ON m.rowid = fts.rowid
     WHERE messages_fts MATCH ? OR s.title LIKE ? ESCAPE '\\'
     ORDER BY s.updated_at DESC LIMIT 50`,
    [ftsQuery, `%${escaped}%`]
  );
}

export interface DbMessage {
  id: string; session_id: string; role: string; content: string; created_at: number;
}

export async function getMessages(sessionId: string): Promise<DbMessage[]> {
  const db = await getDb();
  return db.getAllAsync<DbMessage>(
    'SELECT * FROM messages WHERE session_id = ? ORDER BY created_at ASC', [sessionId]
  );
}

export async function addMessage(sessionId: string, role: string, content: string): Promise<string> {
  const db = await getDb();
  const id = uuid();
  await db.withTransactionAsync(async () => {
    await db.runAsync(
      'INSERT INTO messages (id, session_id, role, content) VALUES (?, ?, ?, ?)',
      [id, sessionId, role, content]
    );
    await db.runAsync('UPDATE sessions SET updated_at = strftime(\'%s\',\'now\') WHERE id = ?', [sessionId]);
  });
  return id;
}

export async function updateMessage(id: string, content: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE messages SET content = ? WHERE id = ?', [content, id]);
}

export async function deleteMessage(id: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('DELETE FROM messages WHERE id = ?', [id]);
}

export interface DbNote {
  id: string; title: string; content: string;
  starred: number; created_at: number; updated_at: number;
}

export async function createNote(title = '无标题'): Promise<string> {
  const db = await getDb();
  const id = uuid();
  await db.runAsync('INSERT INTO notes (id, title) VALUES (?, ?)', [id, title]);
  return id;
}

export async function getNotes(): Promise<DbNote[]> {
  const db = await getDb();
  return db.getAllAsync<DbNote>('SELECT * FROM notes ORDER BY starred DESC, updated_at DESC');
}

export async function getNote(id: string): Promise<DbNote | null> {
  const db = await getDb();
  return db.getFirstAsync<DbNote>('SELECT * FROM notes WHERE id = ?', [id]);
}

export async function updateNote(id: string, title: string, content: string): Promise<void> {
  const db = await getDb();
  await db.runAsync(
    'UPDATE notes SET title = ?, content = ?, updated_at = strftime(\'%s\',\'now\') WHERE id = ?',
    [title, content, id]
  );
}

export async function deleteNote(id: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('DELETE FROM notes WHERE id = ?', [id]);
}

export async function toggleStar(id: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE notes SET starred = 1 - starred WHERE id = ?', [id]);
}

export async function searchNotes(query: string): Promise<DbNote[]> {
  const db = await getDb();
  const ftsQuery = query.split(/\s+/).filter(Boolean).map(t => `"${t}"`).join(' OR ');
  if (!ftsQuery) return [];
  return db.getAllAsync<DbNote>(
    `SELECT n.* FROM notes n
     INNER JOIN notes_fts fts ON n.rowid = fts.rowid
     WHERE notes_fts MATCH ?
     ORDER BY n.updated_at DESC LIMIT 50`,
    [ftsQuery]
  );
}
