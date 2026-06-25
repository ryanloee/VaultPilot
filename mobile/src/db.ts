import * as SQLite from 'expo-sqlite';

let dbPromise: Promise<SQLite.SQLiteDatabase> | null = null;
let ftsSupported = true;

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
  await ensureColumn('notes', 'folder', 'TEXT NOT NULL DEFAULT \'\'');
  await ensureColumn('messages', 'attachments', 'TEXT');

  // #1447: Ensure UNIQUE constraint on pending_syncs.note_id for INSERT OR REPLACE dedup
  const indexes = await db.getAllAsync<{ name: string }>(
    "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_pending_syncs_note_id'"
  );
  if (indexes.length === 0) {
    await db.execAsync('CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_syncs_note_id ON pending_syncs(note_id)');
  }
}

/** Populate FTS tables from existing data (runs once, idempotent via content= sync). */
async function migrateFts(db: SQLite.SQLiteDatabase): Promise<void> {
  // Wrap all inserts in a single transaction so an interruption cannot
  // leave the FTS index in a partially-populated state (#1516).
  // Use NOT IN subqueries against the content tables so that rows missed
  // by a previously interrupted migration are picked up on the next open.
  await db.withTransactionAsync(async () => {
    const msgs = await db.getAllAsync<{ rowid: number; content: string }>(
      'SELECT rowid, content FROM messages WHERE rowid NOT IN (SELECT rowid FROM messages_fts)'
    );
    for (const m of msgs) {
      await db.runAsync('INSERT INTO messages_fts(rowid, content) VALUES (?, ?)', [m.rowid, m.content]);
    }

    const notes = await db.getAllAsync<{ rowid: number; title: string; content: string }>(
      'SELECT rowid, title, content FROM notes WHERE rowid NOT IN (SELECT rowid FROM notes_fts)'
    );
    for (const n of notes) {
      await db.runAsync('INSERT INTO notes_fts(rowid, title, content) VALUES (?, ?, ?)', [n.rowid, n.title, n.content]);
    }
  });
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
          folder TEXT NOT NULL DEFAULT '',
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );
        CREATE TABLE IF NOT EXISTS note_tags (
          note_id TEXT NOT NULL,
          tag TEXT NOT NULL,
          PRIMARY KEY (note_id, tag),
          FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS pending_syncs (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          note_id TEXT NOT NULL,
          action TEXT NOT NULL DEFAULT 'update',
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          retry_count INTEGER NOT NULL DEFAULT 0
        );
      `);
      await db.execAsync('CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);');
      // FTS5 virtual tables — gracefully degrade if device SQLite lacks FTS5
      try {
        await db.execAsync(`
          CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content, content=messages, content_rowid=rowid);
          CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(title, content, content=notes, content_rowid=rowid);
        `);
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
        await migrateFts(db);
      } catch (ftsErr) {
        ftsSupported = false;
        console.warn('[DB] FTS5 not available, falling back to LIKE search:', ftsErr);
      }
      await migrateSchema(db);
      return db;
    })().catch(err => {
      dbPromise = null; // Reset so next call retries instead of caching the failure
      throw err;
    });
  }
  return dbPromise;
}

export function uuid(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) return crypto.randomUUID();
  // Fallback for devices where crypto is unavailable
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => {
    const r = Math.random() * 16 | 0;
    return (c === 'x' ? r : (r & 0x3 | 0x8)).toString(16);
  });
}

/** Escape SQL LIKE special characters (%, _, \\) so they match literally. */
export function escapeLikePattern(pattern: string): string {
  return pattern.replace(/[\\%_]/g, ch => `\\${ch}`);
}

/** Build an FTS5 MATCH query from user input. Splits on whitespace, escapes double quotes, joins with OR. */
export function buildFtsQuery(query: string): string | null {
  const terms = query.split(/\s+/).filter(Boolean).map(t => `"${t.replace(/"/g, '""')}"`);
  const ftsQuery = terms.join(' OR ');
  return ftsQuery || null;
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
  if (!ftsSupported) {
    const escaped = escapeLikePattern(query);
    return db.getAllAsync<DbSession>(
      `SELECT * FROM sessions WHERE title LIKE ? ESCAPE '\' ORDER BY updated_at DESC LIMIT 50`,
      [`%${escaped}%`]
    );
  }
  const ftsQuery = buildFtsQuery(query);
  if (!ftsQuery) return [];
  const escaped = escapeLikePattern(query);
  // FTS5 on message content + LIKE on session title (titles are short, LIKE is fine)
  return db.getAllAsync<DbSession>(
    `SELECT DISTINCT s.* FROM sessions s
     LEFT JOIN messages m ON s.id = m.session_id
     LEFT JOIN messages_fts fts ON m.rowid = fts.rowid
     WHERE messages_fts MATCH ? OR s.title LIKE ? ESCAPE '\'
     ORDER BY s.updated_at DESC LIMIT 50`,
    [ftsQuery, `%${escaped}%`]
  );
}

export interface DbMessage {
  id: string; session_id: string; role: string; content: string; created_at: number;
  attachments?: string; // JSON array of {name: string, type: 'image'|'file'}
}

export async function getMessages(sessionId: string): Promise<DbMessage[]> {
  const db = await getDb();
  return db.getAllAsync<DbMessage>(
    'SELECT * FROM messages WHERE session_id = ? ORDER BY created_at ASC', [sessionId]
  );
}

export async function addMessage(sessionId: string, role: string, content: string, attachments?: { name: string; type: 'image' | 'file' }[]): Promise<string> {
  const db = await getDb();
  const id = uuid();
  const attJson = attachments && attachments.length > 0 ? JSON.stringify(attachments) : null;
  await db.withTransactionAsync(async () => {
    await db.runAsync(
      'INSERT INTO messages (id, session_id, role, content, attachments) VALUES (?, ?, ?, ?, ?)',
      [id, sessionId, role, content, attJson]
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
  starred: number; folder: string; created_at: number; updated_at: number;
}

export async function createNote(title = '无标题', content = '', id?: string): Promise<string> {
  const db = await getDb();
  const noteId = id ?? uuid();
  await db.runAsync('INSERT INTO notes (id, title, content) VALUES (?, ?, ?)', [noteId, title, content]);
  return noteId;
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

export async function getNoteCount(): Promise<number> {
  const db = await getDb();
  const row = await db.getFirstAsync<{ count: number }>('SELECT COUNT(*) as count FROM notes');
  return row?.count ?? 0;
}

export async function getNotes(folder?: string, limit?: number): Promise<DbNote[]> {
  const db = await getDb();
  if (folder !== undefined) {
    if (limit !== undefined) {
      return db.getAllAsync<DbNote>(
        'SELECT * FROM notes WHERE folder = ? ORDER BY starred DESC, updated_at DESC LIMIT ?', [folder, limit]
      );
    }
    return db.getAllAsync<DbNote>(
      'SELECT * FROM notes WHERE folder = ? ORDER BY starred DESC, updated_at DESC', [folder]
    );
  }
  if (limit !== undefined) {
    return db.getAllAsync<DbNote>('SELECT * FROM notes ORDER BY starred DESC, updated_at DESC LIMIT ?', [limit]);
  }
  return db.getAllAsync<DbNote>('SELECT * FROM notes ORDER BY starred DESC, updated_at DESC');
}

export async function getFolders(): Promise<string[]> {
  const db = await getDb();
  const rows = await db.getAllAsync<{ folder: string }>(
    "SELECT DISTINCT folder FROM notes WHERE folder != '' ORDER BY folder"
  );
  return rows.map(r => r.folder);
}

export async function moveToFolder(id: string, folder: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE notes SET folder = ?, updated_at = strftime(\'%s\',\'now\') WHERE id = ?', [folder, id]);
}

export async function getNoteTags(noteId: string): Promise<string[]> {
  const db = await getDb();
  const rows = await db.getAllAsync<{ tag: string }>('SELECT tag FROM note_tags WHERE note_id = ? ORDER BY tag', [noteId]);
  return rows.map(r => r.tag);
}

export async function addTag(noteId: string, tag: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?, ?)', [noteId, tag]);
}

export async function removeTag(noteId: string, tag: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('DELETE FROM note_tags WHERE note_id = ? AND tag = ?', [noteId, tag]);
}

export async function getAllTags(): Promise<string[]> {
  const db = await getDb();
  const rows = await db.getAllAsync<{ tag: string }>('SELECT DISTINCT tag FROM note_tags ORDER BY tag');
  return rows.map(r => r.tag);
}

export async function searchNotes(query: string): Promise<DbNote[]> {
  const db = await getDb();
  if (!ftsSupported) {
    const escaped = escapeLikePattern(query);
    return db.getAllAsync<DbNote>(
      `SELECT * FROM notes WHERE title LIKE ? ESCAPE '\' OR content LIKE ? ESCAPE '\' ORDER BY updated_at DESC LIMIT 50`,
      [`%${escaped}%`, `%${escaped}%`]
    );
  }
  const ftsQuery = buildFtsQuery(query);
  if (!ftsQuery) return [];
  const ftsResults = await db.getAllAsync<DbNote>(
    `SELECT n.* FROM notes n
     INNER JOIN notes_fts fts ON n.rowid = fts.rowid
     WHERE notes_fts MATCH ?
     ORDER BY n.updated_at DESC LIMIT 50`,
    [ftsQuery]
  );
  // Fallback to LIKE search if FTS returns no results (common with CJK text)
  if (ftsResults.length === 0) {
    const escaped = escapeLikePattern(query);
    return db.getAllAsync<DbNote>(
      `SELECT * FROM notes WHERE title LIKE ? ESCAPE '\' OR content LIKE ? ESCAPE '\' ORDER BY updated_at DESC LIMIT 50`,
      [`%${escaped}%`, `%${escaped}%`]
    );
  }
  return ftsResults;
}

export interface GlobalSearchResult {
  type: 'session' | 'note';
  id: string;
  title: string;
  snippet: string;
  updated_at: number;
  sessionId?: string;  // for message matches: which session it belongs to
}

export async function globalSearch(query: string): Promise<GlobalSearchResult[]> {
  const db = await getDb();
  const limit = 20;
  const escaped = escapeLikePattern(query);
  const ftsQuery = ftsSupported ? buildFtsQuery(query) : null;

  // Early return if FTS query is empty (whitespace-only input)
  if (ftsSupported && !ftsQuery) return [];

  // Search notes — FTS with LIKE fallback when FTS returns empty (common with CJK)
  let noteResults: GlobalSearchResult[];
  if (ftsQuery) {
    noteResults = await db.getAllAsync<GlobalSearchResult>(
      `SELECT 'note' as type, n.id, n.title,
              SUBSTR(n.content, 1, 120) as snippet, n.updated_at
       FROM notes n
       INNER JOIN notes_fts fts ON n.rowid = fts.rowid
       WHERE notes_fts MATCH ?
       ORDER BY n.updated_at DESC LIMIT ?`,
      [ftsQuery, limit]
    );
    if (noteResults.length === 0) {
      noteResults = await db.getAllAsync<GlobalSearchResult>(
        `SELECT 'note' as type, id, title,
                SUBSTR(content, 1, 120) as snippet, updated_at
         FROM notes WHERE title LIKE ? ESCAPE '\' OR content LIKE ? ESCAPE '\'
         ORDER BY updated_at DESC LIMIT ?`,
        [`%${escaped}%`, `%${escaped}%`, limit]
      );
    }
  } else {
    noteResults = await db.getAllAsync<GlobalSearchResult>(
      `SELECT 'note' as type, id, title,
              SUBSTR(content, 1, 120) as snippet, updated_at
       FROM notes WHERE title LIKE ? ESCAPE '\' OR content LIKE ? ESCAPE '\'
       ORDER BY updated_at DESC LIMIT ?`,
      [`%${escaped}%`, `%${escaped}%`, limit]
    );
  }

  // Search session messages — FTS with LIKE fallback
  let sessionResults: GlobalSearchResult[];
  if (ftsQuery) {
    sessionResults = await db.getAllAsync<GlobalSearchResult>(
      `SELECT 'session' as type, m.id, s.title,
              SUBSTR(m.content, 1, 120) as snippet, m.created_at as updated_at,
              s.id as sessionId
       FROM messages m
       INNER JOIN messages_fts fts ON m.rowid = fts.rowid
       INNER JOIN sessions s ON m.session_id = s.id
       WHERE messages_fts MATCH ?
       ORDER BY m.created_at DESC LIMIT ?`,
      [ftsQuery, limit]
    );
    if (sessionResults.length === 0) {
      sessionResults = await db.getAllAsync<GlobalSearchResult>(
        `SELECT 'session' as type, m.id, s.title,
                SUBSTR(m.content, 1, 120) as snippet, m.created_at as updated_at,
                s.id as sessionId
         FROM messages m
         INNER JOIN sessions s ON m.session_id = s.id
         WHERE m.content LIKE ? ESCAPE '\' OR s.title LIKE ? ESCAPE '\'
         ORDER BY m.created_at DESC LIMIT ?`,
        [`%${escaped}%`, `%${escaped}%`, limit]
      );
    }
  } else {
    sessionResults = await db.getAllAsync<GlobalSearchResult>(
      `SELECT 'session' as type, m.id, s.title,
              SUBSTR(m.content, 1, 120) as snippet, m.created_at as updated_at,
              s.id as sessionId
       FROM messages m
       INNER JOIN sessions s ON m.session_id = s.id
       WHERE m.content LIKE ? ESCAPE '\' OR s.title LIKE ? ESCAPE '\'
       ORDER BY m.created_at DESC LIMIT ?`,
      [`%${escaped}%`, `%${escaped}%`, limit]
    );
  }

  // Merge and sort by updated_at descending
  return [...noteResults, ...sessionResults]
    .sort((a, b) => b.updated_at - a.updated_at)
    .slice(0, limit);
}

// ── Offline Sync Queue (#1220) ──────────────────────────────

/** Queue a note for sync when back online. */
export async function queuePendingSync(noteId: string, action = 'update'): Promise<void> {
  const db = await getDb();
  // Deduplicate: only one pending entry per note
  await db.runAsync(
    'INSERT OR REPLACE INTO pending_syncs (note_id, action) VALUES (?, ?)',
    [noteId, action]
  );
}

/** Increment retry count for a pending sync entry. */
export async function incrementPendingSyncRetry(noteId: string): Promise<void> {
  const db = await getDb();
  await db.runAsync(
    'UPDATE pending_syncs SET retry_count = retry_count + 1 WHERE note_id = ?',
    [noteId]
  );
}

/** Get retry count for a pending sync entry. */
export async function getPendingSyncRetryCount(noteId: string): Promise<number> {
  const db = await getDb();
  const row = await db.getFirstAsync<{ retry_count: number }>(
    'SELECT retry_count FROM pending_syncs WHERE note_id = ?',
    [noteId]
  );
  return row?.retry_count ?? 0;
}

/** Count of notes waiting to sync. */
export async function getPendingSyncCount(): Promise<number> {
  const db = await getDb();
  const row = await db.getFirstAsync<{ c: number }>('SELECT COUNT(*) as c FROM pending_syncs');
  return row?.c ?? 0;
}

/** Get all pending sync entries. */
export async function getPendingSyncs(): Promise<Array<{ id: number; note_id: string; action: string }>> {
  const db = await getDb();
  return db.getAllAsync<{ id: number; note_id: string; action: string; retry_count: number }>(
    'SELECT * FROM pending_syncs ORDER BY created_at ASC'
  );
}

/** Remove a pending sync entry after successful sync. */
export async function clearPendingSync(noteId: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('DELETE FROM pending_syncs WHERE note_id = ?', [noteId]);
}

/** Remove all pending sync entries (e.g. after full sync). */
export async function clearAllPendingSyncs(): Promise<void> {
  const db = await getDb();
  await db.runAsync('DELETE FROM pending_syncs');
}
