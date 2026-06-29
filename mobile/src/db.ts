import * as SQLite from 'expo-sqlite';
import AsyncStorage from '@react-native-async-storage/async-storage';

let dbPromise: Promise<SQLite.SQLiteDatabase> | null = null;
let ftsSupported = true;

/** Ensure columns added after the initial schema exist on existing installs. */
async function migrateSchema(db: SQLite.SQLiteDatabase): Promise<void> {
  /** Reject identifiers with non-alphanumeric/underscore chars to prevent SQL injection. */
  function assertSafeIdentifier(name: string): void {
    if (!/^[\w]+$/.test(name)) {
      throw new Error(`Unsafe SQL identifier: ${name}`);
    }
  }

  const columns = async (table: string): Promise<Set<string>> => {
    assertSafeIdentifier(table);
    const info = await db.getAllAsync<{ name: string }>(`PRAGMA table_info(${table})`);
    return new Set(info.map(c => c.name));
  };

  const ensureColumn = async (table: string, col: string, decl: string) => {
    assertSafeIdentifier(table);
    assertSafeIdentifier(col);
    const cols = await columns(table);
    if (!cols.has(col)) {
      await db.execAsync(`ALTER TABLE ${table} ADD COLUMN ${col} ${decl}`);
    }
  };

  await ensureColumn('sessions', 'pinned', 'INTEGER DEFAULT 0');
  await ensureColumn('sessions', 'archived', 'INTEGER DEFAULT 0');
  await ensureColumn('notes', 'starred', 'INTEGER DEFAULT 0');
  await ensureColumn('notes', 'folder', 'TEXT NOT NULL DEFAULT \'\'');
  // #2154: Template Snippets — notes flagged as templates (is_template=1) are excluded
  // from regular note listings and only surface in the template picker.
  await ensureColumn('notes', 'is_template', 'INTEGER DEFAULT 0');
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
     WHERE s.id IN (
       SELECT m.session_id FROM messages m
       INNER JOIN messages_fts fts ON m.rowid = fts.rowid
       WHERE messages_fts MATCH ?
     )
     OR s.title LIKE ? ESCAPE '\\'
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
  await db.withTransactionAsync(async () => {
    await db.runAsync('UPDATE messages SET content = ? WHERE id = ?', [content, id]);
    await db.runAsync(
      'UPDATE sessions SET updated_at = strftime(\'%s\',\'now\') WHERE id = (SELECT session_id FROM messages WHERE id = ?)',
      [id]
    );
  });
}

export async function deleteMessage(id: string): Promise<void> {
  const db = await getDb();
  await db.runAsync('DELETE FROM messages WHERE id = ?', [id]);
}

export interface DbNote {
  id: string; title: string; content: string;
  starred: number; folder: string; created_at: number; updated_at: number;
  is_template?: number; // #2154 — 0 (regular note) or 1 (template). Optional for legacy rows pre-migration.
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

/** #2152: resolve a note by its exact title for wikilink / block-reference navigation. */
export async function getNoteByTitle(title: string): Promise<DbNote | null> {
  const db = await getDb();
  return db.getFirstAsync<DbNote>(
    'SELECT * FROM notes WHERE title = ? AND is_template = 0 LIMIT 1',
    [title],
  );
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
  const row = await db.getFirstAsync<{ count: number }>('SELECT COUNT(*) as count FROM notes WHERE is_template = 0');
  return row?.count ?? 0;
}

export async function getNotes(folder?: string, limit?: number): Promise<DbNote[]> {
  const db = await getDb();
  if (folder !== undefined) {
    if (limit !== undefined) {
      return db.getAllAsync<DbNote>(
        'SELECT * FROM notes WHERE folder = ? AND is_template = 0 ORDER BY starred DESC, updated_at DESC LIMIT ?', [folder, limit]
      );
    }
    return db.getAllAsync<DbNote>(
      'SELECT * FROM notes WHERE folder = ? AND is_template = 0 ORDER BY starred DESC, updated_at DESC', [folder]
    );
  }
  if (limit !== undefined) {
    return db.getAllAsync<DbNote>('SELECT * FROM notes WHERE is_template = 0 ORDER BY starred DESC, updated_at DESC LIMIT ?', [limit]);
  }
  return db.getAllAsync<DbNote>('SELECT * FROM notes WHERE is_template = 0 ORDER BY starred DESC, updated_at DESC');
}

/** 只加载 id 和 updated_at，用于同步比较，避免全量 content 导致 OOM (#1668) */
export async function getNoteTimestamps(): Promise<Array<{ id: string; updated_at: number }>> {
  const db = await getDb();
  return db.getAllAsync<{ id: string; updated_at: number }>('SELECT id, updated_at FROM notes');
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

// ── Template Snippets (#2154) ─────────────────────────────
// Templates are regular notes flagged with is_template=1. They are excluded from
// the normal note list/search and only surface in the template picker. Instantiating
// a template clones its body into a fresh note with placeholder variables substituted.

function pad2(n: number): string {
  return n < 10 ? '0' + n : String(n);
}

const WEEKDAYS_ZH = ['日', '一', '二', '三', '四', '五', '六'];

export interface TemplateVars {
  title?: string;
  date?: string;
  time?: string;
  week?: string;
  vault_name?: string;
}

/**
 * Substitute template placeholder variables in content.
 * Supported built-ins: {{title}} {{date}} {{time}} {{week}} {{vault_name}}.
 * Custom fields use {{field:label}} — each unique label maps to a value in `fields`.
 * Unknown / unfilled placeholders are replaced with an empty string so the new note
 * never contains raw {{...}} markers.
 */
export function applyTemplateVariables(
  content: string,
  vars: TemplateVars = {},
  fields: Record<string, string> = {},
): string {
  return content
    .replace(/\{\{title\}\}/g, vars.title ?? '')
    .replace(/\{\{date\}\}/g, vars.date ?? '')
    .replace(/\{\{time\}\}/g, vars.time ?? '')
    .replace(/\{\{week\}\}/g, vars.week ?? '')
    .replace(/\{\{vault_name\}\}/g, vars.vault_name ?? 'VaultPilot')
    .replace(/\{\{field:([^}]+)\}\}/g, (_m, label) => fields[String(label).trim()] ?? '');
}

/** Extract unique custom field labels ({{field:label}}) from template content, in order of appearance. */
export function extractTemplateFields(content: string): string[] {
  const re = /\{\{field:([^}]+)\}\}/g;
  const seen = new Set<string>();
  const out: string[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(content)) !== null) {
    const label = m[1].trim();
    if (label && !seen.has(label)) {
      seen.add(label);
      out.push(label);
    }
  }
  return out;
}

/** Build default built-in variable values for "now". */
export function buildTemplateVars(title: string): Required<TemplateVars> {
  const now = new Date();
  return {
    title,
    date: `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`,
    time: `${pad2(now.getHours())}:${pad2(now.getMinutes())}`,
    week: WEEKDAYS_ZH[now.getDay()],
    vault_name: 'VaultPilot',
  };
}

/** Return all templates, newest first. */
export async function getTemplates(): Promise<DbNote[]> {
  const db = await getDb();
  return db.getAllAsync<DbNote>('SELECT * FROM notes WHERE is_template = 1 ORDER BY updated_at DESC');
}

/** Create a brand-new template note. */
export async function createTemplate(title = '无标题模板', content = ''): Promise<string> {
  const db = await getDb();
  const id = uuid();
  await db.runAsync(
    'INSERT INTO notes (id, title, content, is_template) VALUES (?, ?, ?, 1)',
    [id, title, content],
  );
  return id;
}

/** Copy an existing note's title+content into a new template (non-destructive to the original). */
export async function saveAsTemplate(noteId: string): Promise<string | null> {
  const db = await getDb();
  const src = await db.getFirstAsync<DbNote>('SELECT title, content FROM notes WHERE id = ?', [noteId]);
  if (!src) return null;
  return createTemplate(src.title || '无标题模板', src.content);
}

/** Toggle a note's template flag. */
export async function setTemplateFlag(noteId: string, isTemplate: boolean): Promise<void> {
  const db = await getDb();
  await db.runAsync('UPDATE notes SET is_template = ?, updated_at = strftime(\'%s\',\'now\') WHERE id = ?', [
    isTemplate ? 1 : 0,
    noteId,
  ]);
}

/**
 * Instantiate a template into a fresh regular note with variables substituted.
 * `fieldValues` fills custom {{field:label}} placeholders.
 * Returns the new note's id.
 */
export async function instantiateTemplate(
  templateId: string,
  fieldValues: Record<string, string> = {},
  titleOverride?: string,
): Promise<string> {
  const db = await getDb();
  const tpl = await db.getFirstAsync<DbNote>('SELECT * FROM notes WHERE id = ?', [templateId]);
  if (!tpl) throw new Error('模板不存在');
  const title = (titleOverride ?? tpl.title) || '无标题';
  const vars = buildTemplateVars(title);
  const content = applyTemplateVariables(tpl.content, vars, fieldValues);
  return createNote(title, content);
}

const DEFAULT_TEMPLATES_SEEDED_KEY = 'templates_seeded_v1';

/** Built-in starter templates (会议纪要 / 读书笔记 / 周报). */
export const DEFAULT_TEMPLATES: Array<{ title: string; content: string }> = [
  {
    title: '会议纪要',
    content: [
      '# {{title}}',
      '',
      '- 日期：{{date}} {{week}}',
      '',
      '## 参会人',
      '- ',
      '',
      '## 议题',
      '- ',
      '',
      '## 决议',
      '- ',
      '',
      '## 行动项',
      '- {{field:负责人}}：',
      '',
    ].join('\n'),
  },
  {
    title: '读书笔记',
    content: [
      '# {{title}}',
      '',
      '- 书名：{{field:书名}}',
      '- 作者：{{field:作者}}',
      '- 日期：{{date}}',
      '',
      '## 摘要',
      '',
      '## 核心观点',
      '- ',
      '',
      '## 我的思考',
      '- ',
      '',
    ].join('\n'),
  },
  {
    title: '周报',
    content: [
      '# {{title}} · {{date}}',
      '',
      '## 本周完成',
      '- ',
      '',
      '## 进行中',
      '- ',
      '',
      '## 下周计划',
      '- ',
      '',
      '## 问题与风险',
      '- ',
      '',
    ].join('\n'),
  },
];

/** Idempotently seed built-in templates on first launch. */
export async function ensureDefaultTemplates(): Promise<void> {
  try {
    const flagged = await AsyncStorage.getItem(DEFAULT_TEMPLATES_SEEDED_KEY);
    if (flagged === '1') return;
    const existing = await getTemplates();
    if (existing.length > 0) {
      await AsyncStorage.setItem(DEFAULT_TEMPLATES_SEEDED_KEY, '1');
      return;
    }
    for (const t of DEFAULT_TEMPLATES) {
      await createTemplate(t.title, t.content);
    }
    await AsyncStorage.setItem(DEFAULT_TEMPLATES_SEEDED_KEY, '1');
  } catch (e) {
    console.warn('[DB] ensureDefaultTemplates failed:', e);
  }
}

export async function searchNotes(query: string, folder?: string): Promise<DbNote[]> {
  const db = await getDb();
  const folderFilter = folder !== undefined ? ' AND folder = ?' : '';
  const folderParams = folder !== undefined ? [folder] : [];
  if (!ftsSupported) {
    const escaped = escapeLikePattern(query);
    return db.getAllAsync<DbNote>(
      `SELECT * FROM notes WHERE is_template = 0 AND (title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')${folderFilter} ORDER BY updated_at DESC LIMIT 50`,
      [`%${escaped}%`, `%${escaped}%`, ...folderParams]
    );
  }
  const ftsQuery = buildFtsQuery(query);
  if (!ftsQuery) return [];
  const ftsResults = await db.getAllAsync<DbNote>(
    `SELECT n.* FROM notes n
     INNER JOIN notes_fts fts ON n.rowid = fts.rowid
     WHERE n.is_template = 0 AND notes_fts MATCH ?${folderFilter.replace('folder', 'n.folder')}
     ORDER BY n.updated_at DESC LIMIT 50`,
    [ftsQuery, ...folderParams]
  );
  // Fallback to LIKE search if FTS returns no results (common with CJK text)
  if (ftsResults.length === 0) {
    const escaped = escapeLikePattern(query);
    return db.getAllAsync<DbNote>(
      `SELECT * FROM notes WHERE is_template = 0 AND (title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')${folderFilter} ORDER BY updated_at DESC LIMIT 50`,
      [`%${escaped}%`, `%${escaped}%`, ...folderParams]
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
       WHERE n.is_template = 0 AND notes_fts MATCH ?
       ORDER BY n.updated_at DESC LIMIT ?`,
      [ftsQuery, limit]
    );
    if (noteResults.length === 0) {
      noteResults = await db.getAllAsync<GlobalSearchResult>(
        `SELECT 'note' as type, id, title,
                SUBSTR(content, 1, 120) as snippet, updated_at
         FROM notes WHERE is_template = 0 AND (title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')
         ORDER BY updated_at DESC LIMIT ?`,
        [`%${escaped}%`, `%${escaped}%`, limit]
      );
    }
  } else {
    noteResults = await db.getAllAsync<GlobalSearchResult>(
      `SELECT 'note' as type, id, title,
              SUBSTR(content, 1, 120) as snippet, updated_at
       FROM notes WHERE is_template = 0 AND (title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')
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
  // Deduplicate: only one pending entry per note.
  // #2123: reset retry_count so a fresh edit doesn't inherit past failure counts
  // (otherwise accumulated retries from earlier 5xx failures could push it to
  // MAX_RETRY_ATTEMPTS and silently drop the new edit).
  await db.runAsync(
    'INSERT INTO pending_syncs (note_id, action) VALUES (?, ?) ON CONFLICT(note_id) DO UPDATE SET action = excluded.action, created_at = strftime(\'%s\',\'now\'), retry_count = 0',
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
export async function getPendingSyncs(): Promise<Array<{ id: number; note_id: string; action: string; retry_count: number }>> {
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

// ── Studio (#2166) — 一键生成 vault 交付物 ─────────────────

/** Source context item: a note's title and content for AI consumption. */
export interface StudioSourceNote {
  id: string;
  title: string;
  content: string;
}

/** Fetch multiple notes by ID for use as Studio source context. */
export async function searchNotesByIds(ids: string[]): Promise<StudioSourceNote[]> {
  if (ids.length === 0) return [];
  const db = await getDb();
  const placeholders = ids.map(() => '?').join(',');
  return db.getAllAsync<StudioSourceNote>(
    `SELECT id, title, content FROM notes WHERE id IN (${placeholders}) AND is_template = 0`,
    ids,
  );
}

/**
 * Build a flat text representation of source notes for AI prompt context.
 * Each note is prefixed with its title and a wikilink anchor for citation.
 */
export function buildStudioContext(sources: StudioSourceNote[]): string {
  return sources.map((n, i) => {
    const title = n.title || '无标题';
    return `[Source ${i + 1}]: [[${title}]]\n${n.content || '(空)'}`;
  }).join('\n\n---\n\n');
}
