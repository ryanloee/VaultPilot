import * as SQLite from 'expo-sqlite';

let db: SQLite.SQLiteDatabase | null = null;

export async function getDb(): Promise<SQLite.SQLiteDatabase> {
  if (!db) {
    db = await SQLite.openDatabaseAsync('vaultpilot.db');
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
        folder_id TEXT,
        starred INTEGER DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
        updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
      );
      CREATE TABLE IF NOT EXISTS folders (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
      );
    `);
  }
  return db;
}

function uuid(): string {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
  });
}

/** Escape SQL LIKE special characters (%, _, [) so they match literally. */
function escapeLikePattern(pattern: string): string {
  return pattern.replace(/[%_[]/g, ch => `[${ch}]`);
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
  await db.runAsync('DELETE FROM messages WHERE session_id = ?', [id]);
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
  const escaped = escapeLikePattern(query);
  return db.getAllAsync<DbSession>(
    `SELECT DISTINCT s.* FROM sessions s
     LEFT JOIN messages m ON s.id = m.session_id
     WHERE s.title LIKE ? OR m.content LIKE ?
     ORDER BY s.updated_at DESC LIMIT 50`,
    [`%${escaped}%`, `%${escaped}%`]
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
  await db.runAsync(
    'INSERT INTO messages (id, session_id, role, content) VALUES (?, ?, ?, ?)',
    [id, sessionId, role, content]
  );
  await db.runAsync('UPDATE sessions SET updated_at = strftime(\'%s\',\'now\') WHERE id = ?', [sessionId]);
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
  id: string; title: string; content: string; folder_id: string | null;
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
  const escaped = escapeLikePattern(query);
  return db.getAllAsync<DbNote>(
    'SELECT * FROM notes WHERE title LIKE ? OR content LIKE ? ORDER BY updated_at DESC LIMIT 50',
    [`%${escaped}%`, `%${escaped}%`]
  );
}
